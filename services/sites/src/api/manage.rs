//! The management routes that Edge forwards with the owner claims (ADR-0017, class
//! `management`), and the loopback route of the active manifest that Edge reads to approve it.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::HeaderMap;
use ratatoskr_sites_contracts::{SiteId, SiteManifest};
use serde::Serialize;
use uuid::Uuid;

use super::ApiState;
use super::fault::Fault;

/// The owner claim that Edge mints.
const USER_HEADER: &str = "x-ratatoskr-user-id";

/// The claim of the kind of the session; `anonymous` on Edge's `public` class.
const SESSION_KIND_HEADER: &str = "x-ratatoskr-session-kind";

/// The most sites that `GET /v1/sites` lists, and the most releases and failed builds that
/// `GET /v1/sites/{site}/releases` lists.
pub(crate) const MAX_LISTED: i64 = 100;

/// The owner of a request: the user claim of Edge, unless the claim is anonymous.
pub(crate) fn owner_of(headers: &HeaderMap) -> Option<Uuid> {
    let anonymous = headers
        .get(SESSION_KIND_HEADER)
        .is_some_and(|kind| kind.as_bytes() == b"anonymous");
    if anonymous {
        return None;
    }
    headers
        .get(USER_HEADER)?
        .to_str()
        .ok()
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// One site of `GET /v1/sites`.
#[derive(Debug, Serialize)]
struct SiteState {
    site_id: String,
    submitted_manifest_sha256: Option<String>,
    approved_manifest_sha256: Option<String>,
    approval_state: &'static str,
}

/// `GET /v1/sites`: the sites in ascending `site_id`, at most [`MAX_LISTED`].
pub(crate) async fn list_sites(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<serde_json::Value>, Fault> {
    type Row = (String, Option<String>, Option<String>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "select s.site_id, m.digest, a.approved_digest, a.state from sites.sites s \
         left join sites.site_manifests m on m.site_id = s.site_id and m.is_active \
         left join sites.approval_projection a on a.site_id = s.site_id \
         order by s.site_id limit $1",
    )
    .bind(MAX_LISTED)
    .fetch_all(state.database.pool())
    .await?;
    let sites: Vec<SiteState> = rows
        .into_iter()
        .map(|(site_id, submitted, approved, approval)| SiteState {
            site_id,
            submitted_manifest_sha256: submitted,
            approved_manifest_sha256: approved,
            approval_state: match approval.as_deref() {
                Some("approved") => "approved",
                Some(_) => "withdrawn",
                None => "unapproved",
            },
        })
        .collect();
    Ok(Json(serde_json::json!({ "sites": sites })))
}

/// The answer of a manifest route.
#[derive(Debug, Serialize)]
pub(crate) struct ManifestView {
    site_id: String,
    manifest_sha256: String,
    hostname: String,
    publications: Vec<String>,
    capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_repository: Option<ratatoskr_sites_contracts::GitHubRepositoryBinding>,
}

impl ManifestView {
    fn of(manifest: &SiteManifest, digest: String) -> Self {
        Self {
            site_id: manifest.site_id.as_str().to_owned(),
            manifest_sha256: digest,
            github_repository: manifest.github_repository.clone(),
            hostname: manifest.hostname.as_str().to_owned(),
            publications: manifest
                .publications
                .iter()
                .map(|publication| publication.as_str().to_owned())
                .collect(),
            capabilities: manifest
                .capabilities
                .iter()
                .map(|capability| capability.as_str().to_owned())
                .collect(),
        }
    }
}

/// The active manifest of `site`.
async fn active(state: &ApiState, site: &str) -> Result<Json<ManifestView>, Fault> {
    let row: Option<(String, Vec<u8>)> = sqlx::query_as(
        "select digest, manifest from sites.site_manifests where site_id = $1 and is_active",
    )
    .bind(site)
    .fetch_optional(state.database.pool())
    .await?;
    let (digest, bytes) = row.ok_or(Fault::SiteNotFound)?;
    let manifest = ratatoskr_sites_manifest::parse(&bytes).map_err(|_| Fault::Unavailable)?;
    Ok(Json(ManifestView::of(&manifest, digest)))
}

/// `GET /v1/sites/{site}/manifest`: the active manifest, or 404 `sites.site_not_found`.
pub(crate) async fn get_manifest(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
) -> Result<Json<ManifestView>, Fault> {
    active(&state, &site).await
}

/// `GET /internal/sites/{site}/manifest`: as [`get_manifest`], only to a loopback peer. Edge reads
/// it to approve the active digest.
pub(crate) async fn internal_manifest(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(site): Path<String>,
) -> Result<Json<ManifestView>, Fault> {
    if !peer.ip().is_loopback() {
        return Err(Fault::Forbidden);
    }
    active(&state, &site).await
}

/// `PUT /v1/sites/{site}/manifest`: store a valid manifest by its digest and make it the active
/// one. A new site gets the owner of the request.
pub(crate) async fn put_manifest(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ManifestView>, Fault> {
    let owner = owner_of(&headers).ok_or(Fault::Unauthenticated)?;
    let site = SiteId::parse(&site).map_err(|_| Fault::SiteNotFound)?;
    let manifest = ratatoskr_sites_manifest::parse(&body)
        .map_err(|refusal| Fault::ManifestInvalid(refusal.to_string()))?;
    if manifest.site_id != site {
        return Err(Fault::ManifestInvalid(format!(
            "the manifest is not valid: site_id `{}` is not the site of the route",
            manifest.site_id.as_str()
        )));
    }
    let digest = ratatoskr_sites_manifest::digest(&body);
    let mut transaction = state.database.pool().begin().await?;
    sqlx::query(
        "insert into sites.sites (site_id, owner_id, created_at) values ($1, $2, now()) \
         on conflict (site_id) do nothing",
    )
    .bind(site.as_str())
    .bind(owner)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "update sites.site_manifests set is_active = false where site_id = $1 and is_active \
         and digest <> $2",
    )
    .bind(site.as_str())
    .bind(&digest)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "insert into sites.site_manifests (site_id, digest, manifest, submitted_at, is_active) \
         values ($1, $2, $3, now(), true) on conflict (site_id, digest) do update \
         set submitted_at = excluded.submitted_at, is_active = true",
    )
    .bind(site.as_str())
    .bind(&digest)
    .bind(body.as_ref())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    state
        .servable
        .refresh(&state.database, site.as_str())
        .await?;
    Ok(Json(ManifestView::of(&manifest, digest)))
}

/// One release of the history.
#[derive(Debug, Serialize)]
struct ReleaseEntry {
    release_id: String,
    built_at: String,
    page_count: i32,
}

/// One build that made no release.
#[derive(Debug, Serialize)]
struct FailedBuild {
    operation_id: Uuid,
    failed_at: String,
    code: String,
    message: String,
}

/// The RFC 3339 form of a `timestamptz` column, in UTC.
const RFC3339: &str = "to_char({} at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";

/// `GET /v1/sites/{site}/releases`: the current release id, the releases and the failed builds,
/// newest first, each list at most [`MAX_LISTED`].
pub(crate) async fn list_releases(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
) -> Result<Json<serde_json::Value>, Fault> {
    let pool = state.database.pool();
    let known: bool =
        sqlx::query_scalar("select exists (select 1 from sites.sites where site_id = $1)")
            .bind(&site)
            .fetch_one(pool)
            .await?;
    if !known {
        return Err(Fault::SiteNotFound);
    }
    let current: Option<String> =
        sqlx::query_scalar("select release_id from sites.current_releases where site_id = $1")
            .bind(&site)
            .fetch_optional(pool)
            .await?;
    let releases: Vec<(String, String, i32)> = sqlx::query_as(&format!(
        "select release_id, {}, page_count from sites.releases where site_id = $1 \
         order by built_at desc, release_id limit $2",
        RFC3339.replace("{}", "built_at")
    ))
    .bind(&site)
    .bind(MAX_LISTED)
    .fetch_all(pool)
    .await?;
    let failed: Vec<(Uuid, String, String, String)> = sqlx::query_as(&format!(
        "select operation_id, {}, code, message from sites.failed_builds where site_id = $1 \
         order by failed_at desc, operation_id limit $2",
        RFC3339.replace("{}", "failed_at")
    ))
    .bind(&site)
    .bind(MAX_LISTED)
    .fetch_all(pool)
    .await?;
    let releases: Vec<ReleaseEntry> = releases
        .into_iter()
        .map(|(release_id, built_at, page_count)| ReleaseEntry {
            release_id,
            built_at,
            page_count,
        })
        .collect();
    let failed_builds: Vec<FailedBuild> = failed
        .into_iter()
        .map(|(operation_id, failed_at, code, message)| FailedBuild {
            operation_id,
            failed_at,
            code,
            message,
        })
        .collect();
    Ok(Json(serde_json::json!({
        "current_release_id": current,
        "releases": releases,
        "failed_builds": failed_builds,
    })))
}
