//! The Content Delivery API (workspace spec `site-content-delivery`): `GET` and `HEAD` of
//! `/v1/sites/{site}/content/release`, `.../pages/{slug}`, `.../assets/{sha256}` and
//! `.../search`, with Edge's anonymous claim. Each route serves only the current release of a site
//! whose effective set holds the capability of the route.
//!
//! Every kind of absence (a site that does not exist, a site without an approval or with a
//! withdrawn one, a capability outside the effective set) takes one path: the in-memory set of
//! servable sites says no, and the answer is [`absent`], before any database query.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use ratatoskr_sites_contracts::SiteCapability;
use serde::Deserialize;

use super::ApiState;
use super::fault::Fault;
use crate::search;

/// The `Cache-Control` of the release, page and search routes: at most 60 seconds.
const SHORT: &str = "public, max-age=60";

/// The `Cache-Control` of the asset route: at most 3600 seconds.
const LONG: &str = "public, max-age=3600";

/// The media type of a release, a page and a search answer.
const JSON: &str = "application/json";

/// The one answer of every absence: 404 `sites.site_not_found`, never stored.
pub(crate) fn absent() -> Response {
    not_stored(Fault::SiteNotFound)
}

/// `fault` with `Cache-Control: no-store`.
fn not_stored(fault: Fault) -> Response {
    let mut response = fault.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The `ETag` of `value`: the release id for the release, page and search routes, the digest for
/// the asset route.
fn tag(value: &str) -> String {
    format!("\"{value}\"")
}

/// Whether `If-None-Match` of `headers` matches `etag`.
fn unchanged(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|candidate| candidate.trim().trim_start_matches("W/"))
        .any(|candidate| candidate == etag || candidate == "*")
}

/// A `200` with `body`, or a `304` with no body when `If-None-Match` matches.
fn answer(
    headers: &HeaderMap,
    etag: &str,
    cache: &'static str,
    media_type: &str,
    body: Vec<u8>,
) -> Response {
    let (status, body) = if unchanged(headers, etag) {
        (StatusCode::NOT_MODIFIED, Body::empty())
    } else {
        (StatusCode::OK, Body::from(body))
    };
    let built = Response::builder()
        .status(status)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, cache)
        .header(header::CONTENT_TYPE, media_type)
        .body(body);
    built.unwrap_or_else(|_| Fault::Unavailable.into_response())
}

/// `.../content/release`: the current `sites/release.v1` document.
pub(crate) async fn release(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(serving) = state.servable.serving(&site, SiteCapability::PagesRead) else {
        return absent();
    };
    let etag = tag(&serving.release_id);
    if unchanged(&headers, &etag) {
        return answer(&headers, &etag, SHORT, JSON, Vec::new());
    }
    let document: Result<Option<Vec<u8>>, sqlx::Error> =
        sqlx::query_scalar("select document from sites.releases where release_id = $1")
            .bind(&serving.release_id)
            .fetch_optional(state.database.pool())
            .await;
    match document {
        Ok(Some(document)) => answer(&headers, &etag, SHORT, JSON, document),
        Ok(None) => absent(),
        Err(error) => Fault::from(error).into_response(),
    }
}

/// `.../content/pages/{slug}`: one `sites/page.v1` of the release that the servable set names,
/// or 404 `sites.page_not_found`. Releases are immutable, so a page answer always belongs to the
/// release whose id is its `ETag`.
pub(crate) async fn page(
    State(state): State<Arc<ApiState>>,
    Path((site, slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(serving) = state.servable.serving(&site, SiteCapability::PagesRead) else {
        return absent();
    };
    let etag = tag(&serving.release_id);
    let digest: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(
        "select page_digest from sites.release_pages where release_id = $1 and slug = $2",
    )
    .bind(&serving.release_id)
    .bind(&slug)
    .fetch_optional(state.database.pool())
    .await;
    let digest = match digest {
        Ok(Some(digest)) => digest,
        Ok(None) => return not_stored(Fault::PageNotFound),
        Err(error) => return Fault::from(error).into_response(),
    };
    if unchanged(&headers, &etag) {
        return answer(&headers, &etag, SHORT, JSON, Vec::new());
    }
    match read_blob(&state, &digest).await {
        Some(bytes) => answer(&headers, &etag, SHORT, JSON, bytes),
        None => Fault::Unavailable.into_response(),
    }
}

/// `.../content/assets/{sha256}`: the bytes of one asset of the current release with its media
/// type, or 404 `sites.asset_not_found`.
pub(crate) async fn asset(
    State(state): State<Arc<ApiState>>,
    Path((site, digest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(serving) = state.servable.serving(&site, SiteCapability::AssetsRead) else {
        return absent();
    };
    let media_type: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(
        "select media_type from sites.release_assets where release_id = $1 and digest = $2",
    )
    .bind(&serving.release_id)
    .bind(&digest)
    .fetch_optional(state.database.pool())
    .await;
    let media_type = match media_type {
        Ok(Some(media_type)) => media_type,
        Ok(None) => return not_stored(Fault::AssetNotFound),
        Err(error) => return Fault::from(error).into_response(),
    };
    let etag = tag(&digest);
    if unchanged(&headers, &etag) {
        return answer(&headers, &etag, LONG, &media_type, Vec::new());
    }
    match read_blob(&state, &digest).await {
        Some(bytes) => answer(&headers, &etag, LONG, &media_type, bytes),
        None => Fault::Unavailable.into_response(),
    }
}

/// The parameters of the search route.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    q: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

impl SearchQuery {
    /// The query, the limit and the offset, or `None` when one is outside its bounds: `q` of 1 to
    /// 256 Unicode scalar values, `limit` from 1 to 50 (10 when absent) and a non-negative
    /// `offset` (0 when absent).
    fn bounded(&self) -> Option<(&str, u32, u32)> {
        let q = self.q.as_deref()?;
        if !(1..=search::MAX_QUERY_CHARS).contains(&q.chars().count()) {
            return None;
        }
        let limit = match &self.limit {
            Some(limit) => limit.parse::<u32>().ok()?,
            None => DEFAULT_LIMIT,
        };
        if !(1..=search::MAX_LIMIT).contains(&limit) {
            return None;
        }
        let offset = match &self.offset {
            Some(offset) => offset.parse::<u32>().ok()?,
            None => 0,
        };
        Some((q, limit, offset))
    }
}

/// The number of results when the request names none.
const DEFAULT_LIMIT: u32 = 10;

/// `.../content/search`: search over the current release only. A parameter outside its bounds is
/// 400 `sites.request.invalid`, and no search runs.
pub(crate) async fn search(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
    query: Result<Query<SearchQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Some(serving) = state
        .servable
        .serving(&site, SiteCapability::SearchPublication)
    else {
        return absent();
    };
    let Ok(Query(query)) = query else {
        return Fault::Invalid.into_response();
    };
    let Some((q, limit, offset)) = query.bounded() else {
        return Fault::Invalid.into_response();
    };
    let etag = tag(&serving.release_id);
    if unchanged(&headers, &etag) {
        return answer(&headers, &etag, SHORT, JSON, Vec::new());
    }
    match search::search(&state.database, &serving.release_id, q, limit, offset).await {
        Ok((results, has_more)) => {
            let body = serde_json::json!({ "results": results, "has_more": has_more });
            match serde_json::to_vec(&body) {
                Ok(body) => answer(&headers, &etag, SHORT, JSON, body),
                Err(_) => Fault::Unavailable.into_response(),
            }
        }
        Err(error) => Fault::from(error).into_response(),
    }
}

/// The bytes of the blob `digest` of the sites blob root. A missing or unreadable blob is logged.
async fn read_blob(state: &ApiState, digest: &str) -> Option<Vec<u8>> {
    let path = state.blobs.path(digest)?;
    match tokio::fs::read(&path).await {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::error!(%error, digest, "a stored blob of a release cannot be read");
            None
        }
    }
}
