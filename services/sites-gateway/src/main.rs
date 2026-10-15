//! The `ratatoskr-sites-gateway` deployable.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("check-config") {
        return ratatoskr_sites_gateway::check_config();
    }
    ratatoskr_sites_gateway::run().await
}
