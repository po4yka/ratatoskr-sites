//! The read of the current release of a site and of its pages, for the Content Delivery API.

use ratatoskr_sites_persistence::Database;

use crate::sites_blobs::SitesBlobs;

/// A page and the release it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRead {
    /// The release that holds the page.
    pub release_id: String,
    /// The bytes of the page document.
    pub bytes: Vec<u8>,
}

/// Why a read failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadFailure {
    /// A query failed.
    #[error("a database query of the read failed")]
    Database(#[source] sqlx::Error),
    /// A stored page could not be read.
    #[error("a stored page could not be read")]
    Blob(#[source] std::io::Error),
}

/// The current release of `site_id`: its id and its document.
///
/// # Errors
///
/// [`ReadFailure`] when the query fails.
pub async fn current_release(
    database: &Database,
    site_id: &str,
) -> Result<Option<(String, Vec<u8>)>, ReadFailure> {
    sqlx::query_as(
        "select r.release_id, r.document from sites.current_releases c \
         join sites.releases r on r.release_id = c.release_id where c.site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(database.pool())
    .await
    .map_err(ReadFailure::Database)
}

/// The page `slug` of `site_id` in the release `release_id`, the release whose document the reader
/// fetched. The page is given only while that release is the current release of the site, so a
/// reader never gets a page of another release than the one it fetched; after a switch the answer
/// is not found (workspace spec `site-publications`, "A release becomes current atomically").
///
/// # Errors
///
/// [`ReadFailure`] when the query or the blob read fails.
pub async fn page(
    database: &Database,
    blobs: &SitesBlobs,
    site_id: &str,
    release_id: &str,
    slug: &str,
) -> Result<Option<PageRead>, ReadFailure> {
    let digest: Option<String> = sqlx::query_scalar(
        "select p.page_digest from sites.current_releases c \
         join sites.release_pages p on p.release_id = c.release_id \
         where c.site_id = $1 and c.release_id = $2 and p.slug = $3",
    )
    .bind(site_id)
    .bind(release_id)
    .bind(slug)
    .fetch_optional(database.pool())
    .await
    .map_err(ReadFailure::Database)?;
    let Some(path) = digest.as_deref().and_then(|digest| blobs.path(digest)) else {
        return Ok(None);
    };
    // A stored page never changes, so the bytes are the page of `release_id` also when a newer
    // release became current after the query.
    let bytes = tokio::fs::read(path).await.map_err(ReadFailure::Blob)?;
    Ok(Some(PageRead {
        release_id: release_id.to_owned(),
        bytes,
    }))
}
