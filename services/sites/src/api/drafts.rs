//! The routes of the site owner session (workspace change `add-site-owner-editing-and-live`,
//! design D6): the loopback scope query that Edge asks before an upload, an edit or a conflict
//! read, and the two draft routes that Edge forwards with the claims of a `site_owner` session.
//!
//! Every answer is decided by [`crate::scope`]: a note outside the publications of the site, an
//! unknown `source_ref` and an unknown slug all answer 404 `sites.page_not_found`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse as _, Response};
use ratatoskr_sites_contracts::SourceRef;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ApiState;
use super::fault::Fault;
use super::manage::owner_of;
use crate::scope::{self, ScopeError};

/// The claim of the kind of the session.
const SESSION_KIND_HEADER: &str = "x-ratatoskr-session-kind";

/// The only session kind that a draft route answers.
const SITE_OWNER: &[u8] = b"site_owner";

impl From<ScopeError> for Fault {
    fn from(error: ScopeError) -> Self {
        match error {
            ScopeError::Database(error) => Self::from(error),
            ScopeError::Unavailable => Self::Unavailable,
        }
    }
}

/// The answer of `GET /internal/sites/{site}/sources/{source_ref}`.
#[derive(Debug, Serialize)]
pub(crate) struct Scoped {
    connection_id: Uuid,
    path: String,
}

/// The answer of `GET /internal/sites/{site}/scope`.
#[derive(Debug, Serialize)]
pub(crate) struct InScope {
    source_ref: String,
}

/// The parameters of `GET /internal/sites/{site}/scope`.
#[derive(Debug, Deserialize)]
pub(crate) struct ScopeQuery {
    connection_id: Uuid,
    path: String,
}

/// The answer of a draft route: the source of one note of the site.
#[derive(Debug, Serialize)]
pub(crate) struct Draft {
    source_ref: String,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    markdown: String,
}

/// Refuse a peer that is not on a loopback address, and a user that does not own `site`.
async fn internal_caller(
    state: &ApiState,
    peer: SocketAddr,
    headers: &HeaderMap,
    site: &str,
) -> Result<(), Fault> {
    if !peer.ip().is_loopback() {
        return Err(Fault::Forbidden);
    }
    owns(state, headers, site).await
}

/// Refuse a user that does not own `site` as an unknown site.
async fn owns(state: &ApiState, headers: &HeaderMap, site: &str) -> Result<(), Fault> {
    let user = owner_of(headers).ok_or(Fault::Unauthenticated)?;
    let owner: Option<Uuid> =
        sqlx::query_scalar("select owner_id from sites.sites where site_id = $1")
            .bind(site)
            .fetch_optional(state.database.pool())
            .await?;
    if owner == Some(user) {
        Ok(())
    } else {
        Err(Fault::SiteNotFound)
    }
}

/// `GET /internal/sites/{site}/sources/{source_ref}`: the connection and the path of a note in
/// scope, only to a loopback peer.
pub(crate) async fn internal_source(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((site, source_ref)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Scoped>, Fault> {
    internal_caller(&state, peer, &headers, &site).await?;
    let (connection_id, path) = scoped_source(&state, &site, &source_ref).await?;
    Ok(Json(Scoped {
        connection_id,
        path,
    }))
}

/// `GET /internal/sites/{site}/scope?connection_id=&path=`: the `source_ref` of a note in scope,
/// only to a loopback peer.
pub(crate) async fn internal_scope(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(site): Path<String>,
    headers: HeaderMap,
    query: Result<Query<ScopeQuery>, QueryRejection>,
) -> Result<Json<InScope>, Fault> {
    internal_caller(&state, peer, &headers, &site).await?;
    let Query(query) = query.map_err(|_| Fault::Invalid)?;
    let notes = &state.notes;
    scope::in_scope(
        &state.database,
        notes.index.as_ref(),
        &site,
        query.connection_id,
        &query.path,
    )
    .await?
    .ok_or(Fault::PageNotFound)?;
    let source_ref =
        scope::source_ref(&state.database, &site, query.connection_id, &query.path).await?;
    Ok(Json(InScope { source_ref }))
}

/// The connection and the path of `source_ref` while its note is in the scope of `site`.
async fn scoped_source(
    state: &ApiState,
    site: &str,
    source_ref: &str,
) -> Result<(Uuid, String), Fault> {
    if SourceRef::parse(source_ref).is_err() {
        return Err(Fault::PageNotFound);
    }
    let (connection_id, path) = scope::source_of(&state.database, site, source_ref)
        .await?
        .ok_or(Fault::PageNotFound)?;
    scope::in_scope(
        &state.database,
        state.notes.index.as_ref(),
        site,
        connection_id,
        &path,
    )
    .await?
    .ok_or(Fault::PageNotFound)?;
    Ok((connection_id, path))
}

/// Refuse every session but a `site_owner` session of the owner of `site`.
async fn site_owner(state: &ApiState, headers: &HeaderMap, site: &str) -> Result<(), Fault> {
    let kind = headers.get(SESSION_KIND_HEADER).map(HeaderValue::as_bytes);
    if kind != Some(SITE_OWNER) {
        return Err(Fault::Unauthenticated);
    }
    owns(state, headers, site).await
}

/// `GET /v1/sites/{site}/drafts/sources/{source_ref}`: the source of a note in scope.
pub(crate) async fn source(
    State(state): State<Arc<ApiState>>,
    Path((site, source_ref)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, Fault> {
    site_owner(&state, &headers, &site).await?;
    let (connection_id, path) = scoped_source(&state, &site, &source_ref).await?;
    draft(&state, &site, connection_id, &path).await
}

/// `GET /v1/sites/{site}/drafts/pages/{slug}`: the source of the note of a page of the current
/// release.
pub(crate) async fn page(
    State(state): State<Arc<ApiState>>,
    Path((site, slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, Fault> {
    site_owner(&state, &headers, &site).await?;
    let publications = scope::effective_publications(&state.database, &site).await?;
    let (connection_id, path) = scope::source_of_page(&state.database, &site, &publications, &slug)
        .await?
        .ok_or(Fault::PageNotFound)?;
    draft(&state, &site, connection_id, &path).await
}

/// The draft of `path` of `connection_id`: its current revision and bytes, read while the note is in
/// scope.
async fn draft(
    state: &ApiState,
    site: &str,
    connection_id: Uuid,
    path: &str,
) -> Result<Response, Fault> {
    let notes = &state.notes;
    let file = scope::in_scope(
        &state.database,
        notes.index.as_ref(),
        site,
        connection_id,
        path,
    )
    .await?
    .ok_or(Fault::PageNotFound)?;
    let bytes = notes.blobs.read(&file.revision).await.map_err(|error| {
        tracing::warn!(
            class = error.class(),
            "the bytes of a draft could not be read"
        );
        Fault::Unavailable
    })?;
    let markdown = String::from_utf8(bytes).map_err(|_| {
        tracing::warn!("the bytes of a draft are not UTF-8");
        Fault::Unavailable
    })?;
    let publications = scope::effective_publications(&state.database, site).await?;
    let slug =
        scope::released_slug(&state.database, site, &publications, connection_id, path).await?;
    let source_ref = scope::source_ref(&state.database, site, connection_id, path).await?;
    let body = Draft {
        source_ref,
        revision: file.revision.digest.hex.as_str().to_owned(),
        slug,
        markdown,
    };
    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(body),
    )
        .into_response())
}
