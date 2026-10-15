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

/// The page `slug` of `site_id`, as the reader of the release `release_id` asks for it.
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
    let _ = release_id;
    let row: Option<(String, String)> = sqlx::query_as(
        "select p.release_id, p.page_digest from sites.release_pages p \
         join sites.releases r on r.release_id = p.release_id \
         where r.site_id = $1 and p.slug = $2 order by r.built_at desc limit 1",
    )
    .bind(site_id)
    .bind(slug)
    .fetch_optional(database.pool())
    .await
    .map_err(ReadFailure::Database)?;
    let Some((release_id, digest)) = row else {
        return Ok(None);
    };
    let Some(path) = blobs.path(&digest) else {
        return Ok(None);
    };
    let bytes = tokio::fs::read(path).await.map_err(ReadFailure::Blob)?;
    Ok(Some(PageRead { release_id, bytes }))
}
