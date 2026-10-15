//! The publication routes of the owner (class `management`): `GET .../publications` and `GET`,
//! `PUT`, `DELETE .../publications/{publication}`.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ratatoskr_sites_contracts::{FrontmatterKey, PublicationId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ApiState;
use super::fault::Fault;
use super::manage::MAX_LISTED;
use crate::worker::request_removal_build;

/// The most items of one list of a publication.
const MAX_ITEMS: usize = 64;

/// The longest folder or tag, in characters.
const MAX_ITEM_CHARS: usize = 1024;

/// The body of `PUT .../publications/{publication}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationBody {
    connection_id: Uuid,
    mode: Mode,
    release_mode: ReleaseMode,
    included_folders: Vec<String>,
    excluded_folders: Vec<String>,
    included_tags: Vec<String>,
    frontmatter_allow: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    All,
    Flagged,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReleaseMode {
    Automatic,
    Manual,
}

impl PublicationBody {
    /// Whether every list is within its bounds and every frontmatter name is valid.
    fn is_valid(&self) -> bool {
        let bounded = |items: &[String]| {
            items.len() <= MAX_ITEMS
                && items.iter().all(|item| {
                    !item.is_empty()
                        && item.chars().count() <= MAX_ITEM_CHARS
                        && !item.chars().any(char::is_control)
                })
        };
        bounded(&self.included_folders)
            && bounded(&self.excluded_folders)
            && bounded(&self.included_tags)
            && self.frontmatter_allow.len() <= MAX_ITEMS
            && self
                .frontmatter_allow
                .iter()
                .all(|key| FrontmatterKey::parse(key).is_ok())
    }
}

/// The answer of a publication route.
#[derive(Debug, Serialize)]
pub(crate) struct PublicationView {
    site_id: String,
    publication_id: String,
    connection_id: Uuid,
    mode: String,
    release_mode: String,
    state: String,
    included_folders: Vec<String>,
    excluded_folders: Vec<String>,
    included_tags: Vec<String>,
    frontmatter_allow: Vec<String>,
}

/// One row of [`COLUMNS`].
type Row = (
    String,
    String,
    Uuid,
    String,
    String,
    String,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
);

impl From<Row> for PublicationView {
    fn from(row: Row) -> Self {
        let (site_id, publication_id, connection_id, mode, release_mode, state) =
            (row.0, row.1, row.2, row.3, row.4, row.5);
        Self {
            site_id,
            publication_id,
            connection_id,
            mode,
            release_mode,
            state,
            included_folders: row.6,
            excluded_folders: row.7,
            included_tags: row.8,
            frontmatter_allow: row.9,
        }
    }
}

/// The columns of [`PublicationView`].
const COLUMNS: &str = "site_id, publication_id, connection_id, mode, release_mode, state, \
                       included_folders, excluded_folders, included_tags, frontmatter_allow";

/// `GET /v1/sites/{site}/publications`: the publications in ascending id, at most 100.
pub(crate) async fn list(
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
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "select {COLUMNS} from sites.publications where site_id = $1 \
         order by publication_id limit $2"
    ))
    .bind(&site)
    .bind(MAX_LISTED)
    .fetch_all(pool)
    .await?;
    let publications: Vec<PublicationView> = rows.into_iter().map(PublicationView::from).collect();
    Ok(Json(serde_json::json!({ "publications": publications })))
}

/// `GET /v1/sites/{site}/publications/{publication}`.
pub(crate) async fn read(
    State(state): State<Arc<ApiState>>,
    Path((site, publication)): Path<(String, String)>,
) -> Result<Json<PublicationView>, Fault> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "select {COLUMNS} from sites.publications where site_id = $1 and publication_id = $2"
    ))
    .bind(&site)
    .bind(&publication)
    .fetch_optional(state.database.pool())
    .await?;
    row.map(|row| Json(PublicationView::from(row)))
        .ok_or(Fault::PublicationNotFound)
}

/// `PUT /v1/sites/{site}/publications/{publication}`: create or replace a publication of an
/// existing site. A new vault connection makes a suspended publication active again.
pub(crate) async fn write(
    State(state): State<Arc<ApiState>>,
    Path((site, publication)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<PublicationView>, Fault> {
    let publication = PublicationId::parse(&publication).map_err(|_| Fault::Invalid)?;
    let body: PublicationBody = serde_json::from_slice(&body).map_err(|_| Fault::Invalid)?;
    if !body.is_valid() {
        return Err(Fault::Invalid);
    }
    let existed: bool = sqlx::query_scalar(
        "select exists (select 1 from sites.publications where site_id = $1 \
         and publication_id = $2)",
    )
    .bind(&site)
    .bind(publication.as_str())
    .fetch_one(state.database.pool())
    .await?;
    let written: Option<Row> = sqlx::query_as(&format!(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, state, included_folders, excluded_folders, included_tags, \
         frontmatter_allow, created_at) select site_id, $2, $3, $4, $5, 'active', $6, $7, $8, $9, \
         now() from sites.sites where site_id = $1 \
         on conflict (site_id, publication_id) do update set \
         state = case when sites.publications.connection_id = excluded.connection_id \
         then sites.publications.state else 'active' end, \
         connection_id = excluded.connection_id, mode = excluded.mode, \
         release_mode = excluded.release_mode, included_folders = excluded.included_folders, \
         excluded_folders = excluded.excluded_folders, included_tags = excluded.included_tags, \
         frontmatter_allow = excluded.frontmatter_allow returning {COLUMNS}"
    ))
    .bind(&site)
    .bind(publication.as_str())
    .bind(body.connection_id)
    .bind(match body.mode {
        Mode::All => "all",
        Mode::Flagged => "flagged",
    })
    .bind(match body.release_mode {
        ReleaseMode::Automatic => "automatic",
        ReleaseMode::Manual => "manual",
    })
    .bind(&body.included_folders)
    .bind(&body.excluded_folders)
    .bind(&body.included_tags)
    .bind(&body.frontmatter_allow)
    .fetch_optional(state.database.pool())
    .await?;
    let written = written.ok_or(Fault::SiteNotFound)?;
    if existed {
        // A changed publication can select fewer notes: build their links out of the kept pages.
        removal_build(&state, &site).await?;
    }
    state.servable.refresh(&state.database, &site).await?;
    Ok(Json(PublicationView::from(written)))
}

/// Request a removal build of `site` and wake the worker.
async fn removal_build(state: &ApiState, site: &str) -> Result<(), Fault> {
    if request_removal_build(&state.database, site).await? {
        state.servable.wake(site);
    }
    Ok(())
}

/// `DELETE /v1/sites/{site}/publications/{publication}`: 204, or 404
/// `sites.publication_not_found`. The releases stay as they are; the delivery stops serving the
/// pages and assets that only this publication selected at once.
pub(crate) async fn remove(
    State(state): State<Arc<ApiState>>,
    Path((site, publication)): Path<(String, String)>,
) -> Result<StatusCode, Fault> {
    let removed =
        sqlx::query("delete from sites.publications where site_id = $1 and publication_id = $2")
            .bind(&site)
            .bind(&publication)
            .execute(state.database.pool())
            .await?;
    if removed.rows_affected() == 0 {
        // A retry after a DELETE whose refresh failed: the set is read again before the answer.
        state.servable.refresh(&state.database, &site).await?;
        return Err(Fault::PublicationNotFound);
    }
    // The publication leaves the effective set at once, and with it its pages and assets; a build
    // removes its links from the kept pages.
    removal_build(&state, &site).await?;
    state.servable.refresh(&state.database, &site).await?;
    Ok(StatusCode::NO_CONTENT)
}
