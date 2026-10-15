//! The read-only reader of the notes blob root.
//!
//! `ratatoskr-notes` writes the bytes of each note revision to
//! `<root>/sha256/<first two hex digits>/<other 62 hex digits>`, with the read group
//! `ratatoskr-notes-blobs`. This service is a member of that group and reads the bytes of a
//! `BlobRef` from there. It never writes, renames or deletes anything under the root.
//!
//! The workspace spec `blob-references` requires the reader to verify the owner, the digest
//! algorithm, the digest, the length and a fresh hash before it uses the bytes. [`NotesBlobReader`]
//! checks in that order, and each failure has a stable class ([`NotesBlobError::class`]).

use std::path::PathBuf;

use ratatoskr_identifiers::{BlobRef, DigestAlgorithm};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;

/// The owner of every reference that this reader accepts.
pub const NOTES_BLOB_OWNER: &str = "ratatoskr-notes";

/// Why the bytes of a reference could not be used. No variant carries the bytes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NotesBlobError {
    /// The reference names another owner. Nothing was read.
    #[error("the blob reference is not owned by ratatoskr-notes")]
    WrongOwner,
    /// The reference names a digest algorithm other than SHA-256. Nothing was read.
    #[error("the blob reference uses an unsupported digest algorithm")]
    UnsupportedAlgorithm,
    /// No file is at the path of the digest.
    #[error("the note blob is missing")]
    Missing,
    /// The path of the digest is not a regular file, for example a symbolic link.
    #[error("the note blob is not a regular file")]
    NotAFile,
    /// The file has another length than the reference states.
    #[error("the note blob has another length than its reference")]
    Length,
    /// The bytes do not hash to the digest of the reference.
    #[error("the note blob does not match the digest of its reference")]
    Digest,
    /// The file could not be read, for example because the service is not in the read group.
    #[error("the note blob could not be read")]
    Unreadable(#[source] std::io::Error),
}

impl NotesBlobError {
    /// The stable failure class, for a log line or an operation report.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::WrongOwner => "blob_owner",
            Self::UnsupportedAlgorithm => "blob_algorithm",
            Self::Missing => "blob_missing",
            Self::Length => "blob_length",
            Self::Digest => "blob_digest",
            Self::NotAFile | Self::Unreadable(_) => "blob_unreadable",
        }
    }
}

/// The reader of the notes blob root.
#[derive(Debug, Clone)]
pub struct NotesBlobReader {
    root: PathBuf,
    max_bytes: Option<u64>,
}

impl NotesBlobReader {
    /// A reader of the notes blob root at `root`, without a bound.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_bytes: None,
        }
    }

    /// The same reader, which refuses a reference of more than `max_bytes` bytes.
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    /// The largest reference that the reader reads, when it has a bound.
    #[must_use]
    pub const fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    /// The bytes that `reference` names, after they are verified.
    ///
    /// The owner, the algorithm and the bound of [`NotesBlobReader::with_max_bytes`] are checked
    /// before the root is touched. The length is checked
    /// from the metadata before the file is read, so a file of another length is never read into
    /// memory, and again on the bytes that were read. Then the bytes are hashed. The file is opened
    /// read-only.
    ///
    /// # Errors
    ///
    /// [`NotesBlobError`] with the first check that fails.
    pub async fn read(&self, reference: &BlobRef) -> Result<Vec<u8>, NotesBlobError> {
        if reference.owner_service.as_str() != NOTES_BLOB_OWNER {
            return Err(NotesBlobError::WrongOwner);
        }
        match reference.digest.algorithm {
            DigestAlgorithm::Sha256 => {}
            _ => return Err(NotesBlobError::UnsupportedAlgorithm),
        }
        if self
            .max_bytes
            .is_some_and(|max_bytes| reference.length_bytes > max_bytes)
        {
            return Err(NotesBlobError::Length);
        }
        let hex = reference.digest.hex.as_str();
        // `DigestHex` holds exactly 64 lower-case hex digits, so the split cannot fail.
        let (prefix, rest) = hex.split_at_checked(2).ok_or(NotesBlobError::Missing)?;
        let path = self.root.join("sha256").join(prefix).join(rest);

        let metadata = match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(NotesBlobError::Missing);
            }
            Err(error) => return Err(NotesBlobError::Unreadable(error)),
        };
        if !metadata.is_file() {
            return Err(NotesBlobError::NotAFile);
        }
        if metadata.len() != reference.length_bytes {
            return Err(NotesBlobError::Length);
        }

        let file = tokio::fs::File::open(&path)
            .await
            .map_err(NotesBlobError::Unreadable)?;
        // One byte more than the reference states, so a file that grew after the metadata read
        // is found without reading all of it.
        let mut bytes = Vec::new();
        file.take(reference.length_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .await
            .map_err(NotesBlobError::Unreadable)?;
        if u64::try_from(bytes.len()).ok() != Some(reference.length_bytes) {
            return Err(NotesBlobError::Length);
        }
        if hex_of(&Sha256::digest(&bytes)) != hex {
            return Err(NotesBlobError::Digest);
        }
        Ok(bytes)
    }
}

/// Lower-case hex of `bytes`.
fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
