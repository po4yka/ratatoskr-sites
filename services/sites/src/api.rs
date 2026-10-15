//! The domain API on the loopback listener that Edge forwards `/v1/sites` to (ADR-0017 of
//! `ratatoskr-platform`). It has no route yet.

use std::sync::Arc;

use axum::Router;
use ratatoskr_sites_persistence::Database;

use crate::sites_blobs::SitesBlobs;

/// What the domain handlers share.
#[derive(Debug)]
pub struct ApiState {
    database: Database,
    blobs: SitesBlobs,
}

impl ApiState {
    /// The state of the routes over `database` and the sites blob root `blobs`.
    ///
    /// # Errors
    ///
    /// A query error when the state cannot be read.
    pub async fn load(database: Database, blobs: SitesBlobs) -> Result<Arc<Self>, sqlx::Error> {
        Ok(Arc::new(Self { database, blobs }))
    }
}

/// The domain router.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new().with_state(state)
}
