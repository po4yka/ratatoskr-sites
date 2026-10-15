//! The sites gateway (workspace change `add-site-gateway-hosting`, SITE-026, spec `site-gateway`).
//!
//! One process, `ratatoskr-sites-gateway`, with two listeners: the public listener on
//! `127.0.0.1:8183`, which only `cloudflared` reaches, and the operator plane on `9473`. It reads the
//! sites tree that the sites service writes and never writes it. A request's `Host` selects one
//! approved site; the gateway serves the current deploy of that site.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | `0` | Clean start and clean shutdown |
//! | `1` | Runtime startup failure: telemetry or a listener |
//! | `78` | `EX_CONFIG`: the configuration is unreadable or invalid; nothing was bound |

pub mod admin;
pub mod config;
mod fault;
mod forward;
mod limits;
mod serve;
mod table;

use std::process::ExitCode;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header;
use axum::response::{IntoResponse as _, Response};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use crate::config::GatewayConfig;
use crate::fault::Fault;
use crate::table::HostTable;

/// How often the gateway reads the hostname table again.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// The state of the gateway: its configuration and the hostname table.
#[derive(Debug)]
pub struct Gateway {
    config: GatewayConfig,
    table: RwLock<Arc<HostTable>>,
    manifests: serve::Manifests,
    edge: forward::EdgeClient,
    buckets: limits::Buckets,
}

impl Gateway {
    /// A gateway over `config`, with the hostname table read now.
    #[must_use]
    pub fn new(config: GatewayConfig) -> Arc<Self> {
        let table = HostTable::read(&config.sites.root);
        let edge = forward::client(&config.edge);
        let buckets = limits::Buckets::new(config.limits.rate_per_second, config.limits.burst);
        Arc::new(Self {
            edge,
            buckets,
            config,
            table: RwLock::new(Arc::new(table)),
            manifests: serve::Manifests::default(),
        })
    }

    /// Read the hostname table again from the `site.json` files of the sites tree.
    pub fn rescan(&self) {
        let table = Arc::new(HostTable::read(&self.config.sites.root));
        if let Ok(mut held) = self.table.write() {
            *held = table;
        }
    }

    /// Read the hostname table again every two seconds until `stop`.
    #[must_use]
    pub fn spawn_rescan(self: &Arc<Self>, stop: CancellationToken) -> JoinHandle<()> {
        let gateway = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    () = tokio::time::sleep(RESCAN_INTERVAL) => {}
                }
                let reader = Arc::clone(&gateway);
                if tokio::task::spawn_blocking(move || reader.rescan())
                    .await
                    .is_err()
                {
                    tracing::warn!("a rescan of the sites tree stopped");
                }
            }
        })
    }

    /// The site that `host` selects now.
    fn site(&self, host: &str) -> Option<String> {
        let table = self.table.read().ok()?;
        table.site(host).map(str::to_owned)
    }
}

/// The public router: every request goes to [`handle`].
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new().fallback(handle).with_state(gateway)
}

/// Take a token of the client, select the site of the `Host`, then forward the request to Edge
/// or serve the current deploy.
async fn handle(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    // The limit comes before any work, the host lookup too.
    if let Err(retry_after) = gateway.buckets.take(&parts.headers) {
        return Fault::RateLimited(retry_after).into_response();
    }
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| parts.uri.host())
        .unwrap_or_default();
    let Some(site) = gateway.site(host) else {
        return Fault::UnknownHost.into_response();
    };
    if forward::is_forwarded(parts.uri.path()) {
        return forward::forward(&gateway, &site, parts, body).await;
    }
    serve::static_file(&gateway, &site, &parts).await
}

/// The whole process lifecycle: configuration, telemetry, both listeners, the rescan, and a
/// graceful stop on SIGTERM or SIGINT.
pub async fn run() -> ExitCode {
    let config = match config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{}", error.report());
            return ExitCode::from(error.exit_code());
        }
    };
    let metrics = match admin::init_telemetry(&config.telemetry) {
        Ok(metrics) => metrics,
        Err(reason) => {
            eprintln!("ratatoskr-sites-gateway: refusing to start; {reason}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(config = ?config, version = env!("CARGO_PKG_VERSION"), "effective configuration");
    match admin::serve(config, metrics).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            tracing::error!(%reason, "the process stopped with a failure");
            ExitCode::FAILURE
        }
    }
}

/// `ratatoskr-sites-gateway check-config`: load and validate without binding anything.
#[must_use]
pub fn check_config() -> ExitCode {
    match config::load() {
        Ok(config) => {
            eprintln!("ratatoskr-sites-gateway: configuration is valid.\n{config:#?}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.report());
            ExitCode::from(error.exit_code())
        }
    }
}
