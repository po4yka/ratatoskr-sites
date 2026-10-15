//! The domain API on the loopback listener that Edge forwards `/v1/sites` to (ADR-0017 of
//! `ratatoskr-platform`).
//!
//! Every route reads the claims that Edge mints. Edge removes every reserved header that a client
//! sends, and this listener is reachable only on loopback.

mod fault;
mod manage;
mod publications;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use ratatoskr_sites_persistence::Database;

use crate::servable::Servable;
use crate::sites_blobs::SitesBlobs;
use crate::triggers::Shared;
use fault::Fault;

/// The largest manifest that a submission may carry.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// What the domain handlers share.
#[derive(Debug)]
pub struct ApiState {
    database: Database,
    #[expect(
        dead_code,
        reason = "the delivery routes of SITE-025 task 5.33 read the blobs"
    )]
    blobs: SitesBlobs,
    servable: Arc<Servable>,
}

impl ApiState {
    /// The state of the routes over `shared` and the sites blob root `blobs`.
    ///
    /// # Errors
    ///
    /// A query error when the state cannot be read.
    #[expect(
        clippy::unused_async,
        reason = "the state reads nothing yet; a caller awaits it like the other loads"
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

/// The domain router. A path that no route holds is 404 `sites.route_not_found`. Every route
/// outside `content/` needs the owner claim ([`require_owner`]).
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/v1/sites", get(manage::list_sites))
        .route(
            "/v1/sites/{site}/manifest",
            get(manage::get_manifest).put(manage::put_manifest),
        )
        .route("/v1/sites/{site}/releases", get(manage::list_releases))
        .route("/v1/sites/{site}/publications", get(publications::list))
        .route(
            "/v1/sites/{site}/publications/{publication}",
            get(publications::read)
                .put(publications::write)
                .delete(publications::remove),
        )
        .route(
            "/internal/sites/{site}/manifest",
            get(manage::internal_manifest),
        )
        .route_layer(middleware::from_fn(require_owner))
        .fallback(route_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_MANIFEST_BYTES))
        .with_state(state)
}

/// The second line of ADR-0017: a request with Edge's anonymous claim, or without the owner claim,
/// is 401 `sites.request.unauthenticated` and reaches no handler.
async fn require_owner(request: Request, next: Next) -> Response {
    if manage::owner_of(request.headers()).is_none() {
        return Fault::Unauthenticated.into_response();
    }
    next.run(request).await
}

async fn route_not_found() -> Fault {
    Fault::RouteNotFound
}

async fn method_not_allowed() -> Fault {
    Fault::MethodNotAllowed
}
