//! The index of a vault connection, as the notes internal index route gives it.
//!
//! The route `GET /internal/connections/{connection}/files` of `ratatoskr-notes` (workspace change
//! `add-site-publications`, tasks 4.1 to 4.5) does not exist yet. The build reads the index through
//! this trait; a test gives a stub.

use std::future::Future;

use ratatoskr_sites_publications::IndexedFile;
use uuid::Uuid;

/// The index of a connection could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the notes index of the connection is unavailable")]
pub struct IndexUnavailable;

/// The source of the index of each vault connection.
pub trait NotesIndex: Send + Sync {
    /// Every file of the connection `connection_id`, with its current revision.
    fn files(
        &self,
        connection_id: Uuid,
    ) -> impl Future<Output = Result<Vec<IndexedFile>, IndexUnavailable>> + Send;
}

/// The index in production until the notes internal index route exists (workspace tasks 4.1 to
/// 4.5): it is always unavailable, so a build fails with `sites.notes_index_unavailable` and a note
/// change is delivered again later.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableIndex;

impl NotesIndex for UnavailableIndex {
    async fn files(&self, _connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexUnavailable> {
        Err(IndexUnavailable)
    }
}
