//! The sites gateway (workspace change `add-site-gateway-hosting`, SITE-026, spec `site-gateway`).

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// What the gateway needs to serve.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The sites tree that the sites service writes.
    pub sites_root: PathBuf,
}

/// The state of the gateway.
#[derive(Debug)]
pub struct Gateway {
    config: GatewayConfig,
}

impl Gateway {
    /// A gateway over `config`. The stub of task 6.1 serves nothing.
    #[must_use]
    pub fn new(config: GatewayConfig) -> Arc<Self> {
        Arc::new(Self { config })
    }

    /// Read the hostname table again.
    pub fn rescan(&self) {
        let _ = &self.config;
    }

    /// Read the hostname table again every two seconds until `stop`.
    #[must_use]
    pub fn spawn_rescan(self: &Arc<Self>, stop: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move { stop.cancelled().await })
    }
}

/// The public router.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the stub of task 6.1 serves nothing"
)]
pub fn router(gateway: Arc<Gateway>) -> Router {
    let _ = gateway;
    Router::new().fallback(|| async { "" })
}
