//! The read-only reader of the notes blob root.
//!
//! `ratatoskr-notes` writes the bytes of each note revision to
//! `<root>/sha256/<first two hex digits>/<other 62 hex digits>`, with the read group
//! `ratatoskr-notes-blobs`. This service is a member of that group and reads the bytes of a
//! `BlobRef` from there. It never writes, renames or deletes anything under the root.

use std::path::PathBuf;

use ratatoskr_identifiers::BlobRef;

/// The owner of every reference that this reader accepts.
pub const NOTES_BLOB_OWNER: &str = "ratatoskr-notes";

/// Why the bytes of a reference could not be used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NotesBlobError {
    /// The file could not be read.
    #[error("the note blob could not be read")]
    Unreadable(#[source] std::io::Error),
}

impl NotesBlobError {
    /// The stable failure class, for a log line or an operation report.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Unreadable(_) => "blob_unreadable",
        }
    }
}

/// The reader of the notes blob root.
#[derive(Debug, Clone)]
pub struct NotesBlobReader {
    root: PathBuf,
}

impl NotesBlobReader {
    /// A reader of the notes blob root at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The bytes that `reference` names.
    ///
    /// # Errors
    ///
    /// [`NotesBlobError`] when the bytes cannot be read.
    pub async fn read(&self, reference: &BlobRef) -> Result<Vec<u8>, NotesBlobError> {
        let hex = reference.digest.hex.as_str();
        let path = self
            .root
            .join("sha256")
            .join(hex.get(..2).unwrap_or_default())
            .join(hex.get(2..).unwrap_or_default());
        tokio::fs::read(path)
            .await
            .map_err(NotesBlobError::Unreadable)
    }
}
