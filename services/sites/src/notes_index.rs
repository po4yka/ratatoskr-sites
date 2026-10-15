//! The index of a vault connection, as the notes internal index route gives it.
//!
//! `ratatoskr-notes` serves `GET /internal/connections/{connection}/files` on its loopback domain
//! listener (workspace change `add-site-publications`, tasks 4.1 to 4.5). The build reads the index
//! through this trait; a test gives a stub.

use std::future::Future;
use std::time::Duration;

use ratatoskr_sites_publications::IndexedFile;
use uuid::Uuid;

/// Why the index of a connection could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// The notes service did not answer, answered late, or answered something that is not an
    /// index. A later read can succeed.
    #[error("the notes index of the connection is unavailable")]
    Unavailable,
    /// The connection does not exist in the notes service, or it failed or was removed.
    #[error("the vault connection does not exist")]
    ConnectionNotFound,
}

/// The source of the index of each vault connection.
pub trait NotesIndex: Send + Sync {
    /// Every file of the connection `connection_id`, with its current revision.
    fn files(
        &self,
        connection_id: Uuid,
    ) -> impl Future<Output = Result<Vec<IndexedFile>, IndexError>> + Send;
}

/// The index that the notes internal index route serves.
#[derive(Debug, Clone)]
pub struct HttpNotesIndex {
    base_url: String,
    timeout: Duration,
    max_bytes: usize,
}

impl HttpNotesIndex {
    /// An index read from the notes domain listener at `base_url`, for example
    /// `http://127.0.0.1:8099`, with `timeout` for one read and at most `max_bytes` of answer.
    #[must_use]
    pub fn new(base_url: &str, timeout: Duration, max_bytes: usize) -> Self {
        Self {
            base_url: base_url.to_owned(),
            timeout,
            max_bytes,
        }
    }
}

impl NotesIndex for HttpNotesIndex {
    async fn files(&self, _connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexError> {
        let _ = (&self.base_url, self.timeout, self.max_bytes);
        Err(IndexError::Unavailable)
    }
}

/// The index in production until the notes internal index route exists (workspace tasks 4.1 to
/// 4.5): it is always unavailable, so a build fails with `sites.notes_index_unavailable` and a note
/// change is delivered again later.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableIndex;

impl NotesIndex for UnavailableIndex {
    async fn files(&self, _connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexError> {
        Err(IndexError::Unavailable)
    }
}
