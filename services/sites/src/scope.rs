//! The notes of a site that its owner session may read and edit (workspace change
//! `add-site-owner-editing-and-live`, design D6; spec `site-owner-editing`).
//!
//! A note is in the scope of a site while an active publication of the effective set of the site,
//! over the connection of the note, selects the note in the notes index now. The scope is decided
//! again on each request, so a note that is not in a release yet is in scope, and a note that a
//! publication stops selecting leaves the scope at once.
//!
//! Each note in scope has one opaque `source_ref` per site: `src_` and the 32 hex digits of a random
//! UUID, kept in `sites.source_refs`. The name holds no path, no time and no hash of a path. The
//! table only names notes; it never decides the scope.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ratatoskr_sites_contracts::{PublicationId, SiteId};
use ratatoskr_sites_persistence::Database;
use ratatoskr_sites_publications::{
    IndexedFile, Publication, PublicationMode, assign_slugs, select,
};
use uuid::Uuid;

use crate::effective::effective_set;
use crate::notes_blobs::NotesBlobReader;
use crate::notes_index::{IndexError, NotesIndex};

/// The answer of [`IndexSource::files_of`].
type FilesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<IndexedFile>, IndexError>> + Send + 'a>>;

/// [`NotesIndex`] in a form that a route state can hold, for the index of production and of a test.
pub trait IndexSource: Send + Sync {
    /// Every file of the connection `connection_id`, with its current revision.
    fn files_of(&self, connection_id: Uuid) -> FilesFuture<'_>;
}

impl<I: NotesIndex> IndexSource for I {
    fn files_of(&self, connection_id: Uuid) -> FilesFuture<'_> {
        Box::pin(self.files(connection_id))
    }
}

/// The notes index and the notes blob root, as the routes of the owner session read them.
#[derive(Clone)]
pub struct NotesAccess {
    pub(crate) index: Arc<dyn IndexSource>,
    pub(crate) blobs: NotesBlobReader,
}

impl NotesAccess {
    /// The notes of `index` and of the blob root of `blobs`.
    #[must_use]
    pub fn new<I: NotesIndex + 'static>(index: I, blobs: NotesBlobReader) -> Self {
        Self {
            index: Arc::new(index),
            blobs,
        }
    }
}

impl std::fmt::Debug for NotesAccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotesAccess")
            .field("blobs", &self.blobs)
            .finish_non_exhaustive()
    }
}

/// Why the scope could not be decided.
#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    /// A query failed.
    #[error("a query of the scope failed")]
    Database(#[from] sqlx::Error),
    /// The notes index of the connection could not be read.
    #[error("the notes index is unavailable")]
    Unavailable,
}

/// The indexed file of `path` of `connection_id` when the note is in the scope of `site_id`, with
/// its current revision; `None` when it is not.
///
/// # Errors
///
/// [`ScopeError`] when a query fails or the notes index cannot be read.
pub async fn in_scope(
    database: &Database,
    index: &dyn IndexSource,
    site_id: &str,
    connection_id: Uuid,
    path: &str,
) -> Result<Option<IndexedFile>, ScopeError> {
    let publications = scope_publications(database, site_id, connection_id).await?;
    if publications.is_empty() {
        return Ok(None);
    }
    let files = match index.files_of(connection_id).await {
        Ok(files) => files,
        Err(IndexError::ConnectionNotFound) => return Ok(None),
        Err(IndexError::Unavailable) => return Err(ScopeError::Unavailable),
    };
    let selected = publications
        .iter()
        .any(|publication| select(publication, &files).contains(path));
    Ok(selected
        .then(|| files.into_iter().find(|file| file.path == path))
        .flatten())
}

/// The active publications of the effective set of `site_id` over `connection_id`.
async fn scope_publications(
    database: &Database,
    site_id: &str,
    connection_id: Uuid,
) -> Result<Vec<Publication>, sqlx::Error> {
    type Row = (String, String, Vec<String>, Vec<String>, Vec<String>);
    let effective = effective_set(database, site_id).await?;
    let Ok(site) = SiteId::parse(site_id) else {
        return Ok(Vec::new());
    };
    let rows: Vec<Row> = sqlx::query_as(
        "select publication_id, mode, included_folders, excluded_folders, included_tags \
         from sites.publications where site_id = $1 and connection_id = $2 \
         and state = 'active' order by publication_id",
    )
    .bind(site_id)
    .bind(connection_id)
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(publication_id, ..)| effective.publications.contains(publication_id))
        .filter_map(|(publication_id, mode, included, excluded, tags)| {
            Some(Publication {
                site_id: site.clone(),
                publication_id: PublicationId::parse(&publication_id).ok()?,
                mode: if mode == "all" {
                    PublicationMode::All
                } else {
                    PublicationMode::Flagged
                },
                included_folders: included,
                excluded_folders: excluded,
                included_tags: tags,
            })
        })
        .collect())
}

/// The `source_ref` of `path` of `connection_id` in `site_id`, minted the first time.
///
/// # Errors
///
/// The query error when the name cannot be written or read.
pub async fn source_ref(
    database: &Database,
    site_id: &str,
    connection_id: Uuid,
    path: &str,
) -> Result<String, sqlx::Error> {
    let minted = format!("src_{}", Uuid::new_v4().simple());
    sqlx::query(
        "insert into sites.source_refs (source_ref, site_id, connection_id, path) \
         values ($1, $2, $3, $4) on conflict (site_id, connection_id, path) do nothing",
    )
    .bind(&minted)
    .bind(site_id)
    .bind(connection_id)
    .bind(path)
    .execute(database.pool())
    .await?;
    sqlx::query_scalar(
        "select source_ref from sites.source_refs \
         where site_id = $1 and connection_id = $2 and path = $3",
    )
    .bind(site_id)
    .bind(connection_id)
    .bind(path)
    .fetch_one(database.pool())
    .await
}

/// The connection and the path that `source_ref` names in `site_id`. It says nothing about the
/// scope: the caller asks [`in_scope`].
///
/// # Errors
///
/// The query error when the name cannot be read.
pub async fn source_of(
    database: &Database,
    site_id: &str,
    source_ref: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "select connection_id, path from sites.source_refs \
         where site_id = $1 and source_ref = $2",
    )
    .bind(site_id)
    .bind(source_ref)
    .fetch_optional(database.pool())
    .await
}

/// The publications of the effective set of `site_id`.
///
/// # Errors
///
/// The query error when the set cannot be read.
pub async fn effective_publications(
    database: &Database,
    site_id: &str,
) -> Result<BTreeSet<String>, sqlx::Error> {
    Ok(effective_set(database, site_id).await?.publications)
}

/// The slug of the note `path` of `connection_id` in the current release of `site_id`, when the
/// release has its page and a publication of `publications` serves it.
///
/// # Errors
///
/// The query error when the release cannot be read.
pub async fn released_slug(
    database: &Database,
    site_id: &str,
    publications: &BTreeSet<String>,
    connection_id: Uuid,
    path: &str,
) -> Result<Option<String>, sqlx::Error> {
    let file: Option<serde_json::Value> = sqlx::query_scalar(
        "select s.file from sites.release_sources s \
         join sites.current_releases c on c.release_id = s.release_id \
         where c.site_id = $1 and s.connection_id = $2 and s.path = $3",
    )
    .bind(site_id)
    .bind(connection_id)
    .bind(path)
    .fetch_optional(database.pool())
    .await?;
    let Some(slug) = file.and_then(slug_of) else {
        return Ok(None);
    };
    Ok(serves_page(database, site_id, publications, &slug)
        .await?
        .then_some(slug))
}

/// The connection and the path of the note of the page `slug` of the current release of
/// `site_id`, when a publication of `publications` serves the page.
///
/// # Errors
///
/// The query error when the release cannot be read.
pub async fn source_of_page(
    database: &Database,
    site_id: &str,
    publications: &BTreeSet<String>,
    slug: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    if !serves_page(database, site_id, publications, slug).await? {
        return Ok(None);
    }
    let rows: Vec<(Uuid, String, serde_json::Value)> = sqlx::query_as(
        "select s.connection_id, s.path, s.file from sites.release_sources s \
         join sites.current_releases c on c.release_id = s.release_id \
         where c.site_id = $1 order by s.connection_id, s.path",
    )
    .bind(site_id)
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .into_iter()
        .find(|(_, _, file)| slug_of(file.clone()).as_deref() == Some(slug))
        .map(|(connection_id, path, _)| (connection_id, path)))
}

/// Whether the current release of `site_id` has the page `slug` and a publication of
/// `publications` serves it.
async fn serves_page(
    database: &Database,
    site_id: &str,
    publications: &BTreeSet<String>,
    slug: &str,
) -> Result<bool, sqlx::Error> {
    let owners: Option<Vec<String>> = sqlx::query_scalar(
        "select p.publication_ids from sites.release_pages p \
         join sites.current_releases c on c.release_id = p.release_id \
         where c.site_id = $1 and p.slug = $2",
    )
    .bind(site_id)
    .bind(slug)
    .fetch_optional(database.pool())
    .await?;
    Ok(owners.is_some_and(|owners| owners.iter().any(|owner| publications.contains(owner))))
}

/// The slug of the stored source `file` of a release, `None` for an attachment.
fn slug_of(file: serde_json::Value) -> Option<String> {
    let file: IndexedFile = serde_json::from_value(file).ok()?;
    if !file.is_markdown() {
        return None;
    }
    let slugs = assign_slugs(&[&file]).ok()?;
    slugs
        .into_keys()
        .next()
        .map(|slug| slug.as_str().to_owned())
}
