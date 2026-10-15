//! The `ratatoskr-sites` service library: the process lifecycle and the routes.
//!
//! The binary's `main` calls [`run`] or [`check_config`] and nothing else. The library is separate
//! from the binary so that integration tests drive the real routes.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | `0` | Clean start and clean shutdown |
//! | `1` | Runtime startup failure: telemetry, the database, the schema, or a listener |
//! | `78` | `EX_CONFIG`: the configuration is unreadable or invalid; nothing was bound |

pub mod admin;
pub mod api;
pub mod config;
pub mod consumer;
pub mod effective;
pub mod events;
pub mod notes_blobs;
pub mod notes_index;
pub mod outbox;
mod provenance;
pub mod release_build;
pub mod release_read;
pub mod scope;
pub mod search;
pub mod servable;
pub mod sites_blobs;
pub mod triggers;
pub mod worker;

use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use ratatoskr_sites_deploys::{DeployStore, Limits};
use ratatoskr_sites_persistence::Database;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::admin::{RuntimeState, admin_router};
use crate::api::ApiState;
use crate::config::{LogFormat, SitesConfig, TelemetryConfig};
use crate::consumer::{BusTarget, Durable, Durables};
use crate::notes_blobs::NotesBlobReader;
use crate::notes_index::HttpNotesIndex;

/// How long one read of the notes index of a connection may take.
const NOTES_INDEX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The largest answer of the notes index of a connection: 32 MiB, the index of a vault of tens of
/// thousands of notes.
const NOTES_INDEX_MAX_BYTES: usize = 32 * 1024 * 1024;
use crate::outbox::OutboxRelay;
use crate::release_build::ReleaseBuilder;
use crate::scope::NotesAccess;
use crate::servable::Servable;
use crate::sites_blobs::SitesBlobs;
use crate::triggers::{Consumers, Shared};

/// The whole process lifecycle.
///
/// 1. Load the configuration; on failure write the report to stderr and exit 78.
/// 2. Initialise logging and the metrics recorder; on failure exit 1.
/// 3. Connect the database and apply the schema; on failure exit 1.
/// 4. Bind the operator and the domain listeners; on failure exit 1.
/// 5. Start the outbox relay, the consumers of the durables and the build worker when
///    `RATATOSKR__BUS__URL` is set.
/// 6. Serve until SIGTERM or SIGINT, then let in-flight requests finish within the grace window,
///    and stop the relay.
pub async fn run() -> ExitCode {
    let config = match config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{}", error.report());
            return ExitCode::from(error.exit_code());
        }
    };
    let metrics = match init_telemetry(&config.telemetry) {
        Ok(metrics) => metrics,
        Err(reason) => {
            eprintln!("ratatoskr-sites: refusing to start; {reason}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(config = ?config, version = env!("CARGO_PKG_VERSION"), "effective configuration");

    match serve(&config, metrics).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            tracing::error!(%reason, "the process stopped with a failure");
            ExitCode::FAILURE
        }
    }
}

/// `ratatoskr-sites check-config`: load and validate without binding anything, and exit 0 or 78.
///
/// Both outputs go to stderr: the workspace forbids writing to stdout.
#[must_use]
pub fn check_config() -> ExitCode {
    match config::load() {
        Ok(config) => {
            // Safe by type: the database URL renders as `[REDACTED]`.
            eprintln!("ratatoskr-sites: configuration is valid.\n{config:#?}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.report());
            ExitCode::from(error.exit_code())
        }
    }
}

fn init_telemetry(telemetry: &TelemetryConfig) -> Result<PrometheusHandle, String> {
    let filter = EnvFilter::try_new(&telemetry.log_filter)
        .map_err(|_| "the log filter could not be parsed".to_owned())?;
    let registry = tracing_subscriber::registry().with(filter);
    let installed = match telemetry.log_format {
        LogFormat::Text => registry
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .try_init(),
        LogFormat::Json => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::io::stderr),
            )
            .try_init(),
    };
    installed.map_err(|_| "the log subscriber could not be installed".to_owned())?;
    PrometheusBuilder::new()
        .install_recorder()
        .map_err(|_| "the metrics recorder could not be installed".to_owned())
}

async fn serve(config: &SitesConfig, metrics: PrometheusHandle) -> Result<(), String> {
    let database = Database::connect(
        &config.database.url,
        config.database.max_connections,
        config.database.acquire_timeout(),
    )
    .await
    .map_err(|error| {
        format!("the database could not be reached: {error}; check RATATOSKR__DATABASE__URL")
    })?;
    database
        .apply_schema()
        .await
        .map_err(|error| format!("the sites schema could not be applied: {error}"))?;

    let shared = Shared {
        database: database.clone(),
        servable: Servable::load(&database)
            .await
            .map_err(|error| format!("the servable sites could not be read: {error}"))?,
        sites_tree: Some(config.deploys.root.clone()),
    };
    let notes = NotesAccess::new(
        notes_index(config),
        NotesBlobReader::new(&config.blobs.notes_root)
            .with_max_bytes(release_build::MAX_NOTE_BYTES),
    );
    let deploys = DeployStore::new(
        database.clone(),
        config.deploys.root.clone(),
        Limits {
            max_bundle_bytes: config.deploys.max_bundle_bytes,
            retention: config.deploys.retention,
        },
    );
    let api_state = ApiState::load(
        &shared,
        SitesBlobs::new(&config.blobs.root),
        notes,
        (deploys, config.deploys.max_bundle_bytes),
    )
    .await
    .map_err(|error| format!("the domain state could not be read: {error}"))?;
    let runtime = Arc::new(RuntimeState::new(database.clone()));
    let admin = bind(config.admin.bind, "operator").await?;
    let api = bind(config.api.listen_address, "domain").await?;

    let (stop_sender, stop_receiver) = tokio::sync::watch::channel(false);
    let admin_task = spawn_server(
        admin,
        admin_router(Arc::clone(&runtime), metrics),
        stop_receiver.clone(),
    );
    let api_task = spawn_server(api, api::router(api_state), stop_receiver);

    // The relay publishes what the outbox holds, also the events of an earlier process. The
    // consumers of the durables and the build worker run only with a bus too.
    let bus_stop = tokio_util::sync::CancellationToken::new();
    let mut bus_tasks = Vec::new();
    if let Some(url) = &config.bus.url {
        let relay = OutboxRelay::new(database.clone(), url.clone(), config.bus.clone());
        bus_tasks.push(tokio::spawn(relay.run(bus_stop.clone())));
        bus_tasks.extend(start_pipeline(config, &shared, url, &bus_stop));
    } else {
        tracing::info!(
            "RATATOSKR__BUS__URL is not set; the outbox is not relayed and no durable is read"
        );
    }

    runtime.mark_started();
    tracing::info!(
        admin = %config.admin.bind,
        api = %config.api.listen_address,
        "startup complete"
    );

    shutdown_signal().await;
    runtime.mark_stopping();
    tracing::info!("stop signal received; draining");
    let _ = stop_sender.send(true);

    let grace = std::time::Duration::from_secs(config.shutdown.grace_seconds);
    let drained = tokio::time::timeout(grace, async {
        let _ = admin_task.await;
        let _ = api_task.await;
    })
    .await
    .is_ok();
    bus_stop.cancel();
    // A build that runs at the stop ends first; the grace window bounds the wait.
    let _ = tokio::time::timeout(grace, async {
        for task in bus_tasks {
            let _ = task.await;
        }
    })
    .await;
    tracing::info!(graceful = drained, "the process stopped");
    database.close().await;
    Ok(())
}

/// The consumers of the four durables, and the build worker, which read the notes internal index
/// route at `RATATOSKR__NOTES__BASE_URL`.
fn start_pipeline(
    config: &SitesConfig,
    shared: &Shared,
    url: &str,
    stop: &tokio_util::sync::CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    let index = notes_index(config);
    let builder = ReleaseBuilder::new(
        shared.database.clone(),
        index.clone(),
        NotesBlobReader::new(&config.blobs.notes_root),
        config.blobs.root.clone(),
    );
    let target = BusTarget {
        url: url.to_owned(),
        bus: config.bus.clone(),
    };
    triggers::start(
        shared,
        index,
        builder,
        &Consumers {
            target: &target,
            specs: &Durables::production(),
            durables: &[
                Durable::ReleaseBuilds,
                Durable::NoteChanges,
                Durable::VaultDisconnections,
                Durable::Approvals,
            ],
        },
        stop,
    )
}

/// The client of the notes internal index route at `RATATOSKR__NOTES__BASE_URL`.
fn notes_index(config: &SitesConfig) -> HttpNotesIndex {
    HttpNotesIndex::new(
        &config.notes.base_url,
        NOTES_INDEX_TIMEOUT,
        NOTES_INDEX_MAX_BYTES,
    )
}

async fn bind(address: std::net::SocketAddr, name: &str) -> Result<TcpListener, String> {
    TcpListener::bind(address)
        .await
        .map_err(|error| format!("the {name} listener could not bind {address}: {error}"))
}

fn spawn_server(
    listener: TcpListener,
    router: Router,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let served = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = stop.wait_for(|stopping| *stopping).await;
        })
        .await;
        if let Err(error) = served {
            tracing::error!(%error, "a listener stopped with an error");
        }
    })
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(%error, "SIGTERM cannot be observed; only SIGINT stops the process");
                let _ = interrupt.await;
                return;
            }
        };
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = interrupt.await;
    }
}
