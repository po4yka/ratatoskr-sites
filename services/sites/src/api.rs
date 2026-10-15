//! The domain API on the loopback listener that Edge forwards `/v1/sites` to (ADR-0017 of
//! `ratatoskr-platform`). It has no route yet.

use std::sync::Arc;

use axum::Router;
use ratatoskr_sites_persistence::Database;

use crate::servable::Servable;
use crate::sites_blobs::SitesBlobs;
use crate::triggers::Shared;

/// What the domain handlers share.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "the routes of SITE-025 tasks 5.27 and 5.33 read the state"
)]
pub struct ApiState {
    database: Database,
    blobs: SitesBlobs,
    servable: Arc<Servable>,
}

impl ApiState {
    /// The state of the routes over `database` and the sites blob root `blobs`.
    ///
    /// # Errors
    ///
    /// A query error when the state cannot be read.
    #[expect(
        clippy::unused_async,
        reason = "SITE-025 task 5.27 reads the servable sites here"
    )]
    pub async fn load(shared: &Shared, blobs: SitesBlobs) -> Result<Arc<Self>, sqlx::Error> {
        Ok(Arc::new(Self {
            database: shared.database.clone(),
            blobs,
            servable: Arc::clone(&shared.servable),
        }))
    }

    /// The servable sites.
    #[must_use]
    pub fn servable(&self) -> &Servable {
        &self.servable
    }
}

/// The domain router.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new().with_state(state)
}
