//! The operator plane, the telemetry and the two listeners.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::config::{GatewayConfig, LogFormat, TelemetryConfig};
use crate::{Gateway, router};

/// The facts that readiness is computed from.
#[derive(Debug)]
pub struct RuntimeState {
    sites_root: std::path::PathBuf,
    started: AtomicBool,
    stopping: AtomicBool,
}

impl RuntimeState {
    /// A state over the sites tree `sites_root` that is not ready until
    /// [`RuntimeState::mark_started`].
    #[must_use]
    pub const fn new(sites_root: std::path::PathBuf) -> Self {
        Self {
            sites_root,
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        }
    }

    /// Both listeners are bound.
    pub fn mark_started(&self) {
        self.started.store(true, Ordering::SeqCst);
    }

    /// A stop signal arrived.
    pub fn mark_stopping(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct AdminState {
    runtime: Arc<RuntimeState>,
    metrics: PrometheusHandle,
}

/// The operator router: `/health/live`, `/health/ready`, `/metrics`, `/version`.
pub fn admin_router(runtime: Arc<RuntimeState>, metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route("/health/ready", get(ready))
        .route("/metrics", get(render_metrics))
        .route("/version", get(|| async { env!("CARGO_PKG_VERSION") }))
        .with_state(AdminState { runtime, metrics })
}

/// 200 only after startup, before the stop signal, and while the sites tree can be read.
async fn ready(State(state): State<AdminState>) -> StatusCode {
    let runtime = &state.runtime;
    if !runtime.started.load(Ordering::SeqCst) || runtime.stopping.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    match tokio::fs::read_dir(&runtime.sites_root).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn render_metrics(State(state): State<AdminState>) -> impl IntoResponse {
    state.metrics.render()
}

/// Install the log subscriber on stderr and the metrics recorder.
///
/// # Errors
///
/// A text that names what could not be installed.
pub fn init_telemetry(telemetry: &TelemetryConfig) -> Result<PrometheusHandle, String> {
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

/// Bind both listeners and serve until SIGTERM or SIGINT.
///
/// # Errors
///
/// A text that names the listener that could not bind.
pub async fn serve(config: GatewayConfig, metrics: PrometheusHandle) -> Result<(), String> {
    let runtime = Arc::new(RuntimeState::new(config.sites.root.clone()));
    let admin = bind(config.admin.bind, "operator").await?;
    let public = bind(config.public.listen_address, "public").await?;
    let gateway = Gateway::new(config);
    let stop = CancellationToken::new();
    let rescan = gateway.spawn_rescan(stop.clone());
    let admin_task = spawn_server(
        admin,
        admin_router(Arc::clone(&runtime), metrics),
        stop.clone(),
    );
    let public_task = spawn_server(public, router(gateway), stop.clone());
    runtime.mark_started();
    tracing::info!("startup complete");
    shutdown_signal().await;
    runtime.mark_stopping();
    tracing::info!("stop signal received; draining");
    stop.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let _ = public_task.await;
        let _ = admin_task.await;
        let _ = rescan.await;
    })
    .await;
    tracing::info!("the process stopped");
    Ok(())
}

async fn bind(address: std::net::SocketAddr, name: &str) -> Result<TcpListener, String> {
    TcpListener::bind(address)
        .await
        .map_err(|error| format!("the {name} listener could not bind {address}: {error}"))
}

fn spawn_server(
    listener: TcpListener,
    router: Router,
    stop: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let served = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move { stop.cancelled().await })
        .await;
        if let Err(error) = served {
            tracing::error!(%error, "a listener stopped with an error");
        }
    })
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        tracing::error!("SIGTERM cannot be observed; only SIGINT stops the process");
        let _ = interrupt.await;
        return;
    };
    tokio::select! {
        _ = interrupt => {}
        _ = terminate.recv() => {}
    }
}
