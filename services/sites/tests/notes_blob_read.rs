//! SITE-025 task 5.8: a note blob is verified before it is used (workspace spec `blob-references`,
//! requirement "A consumer may read a producer's blob root read-only").

use std::path::Path;

use ratatoskr_identifiers::{
    BlobOwner, BlobRef, ContentDigest, DigestAlgorithm, DigestHex, MediaType,
};
use ratatoskr_sites::notes_blobs::{NotesBlobError, NotesBlobReader};

/// The bytes of a note, and their SHA-256, computed once with `shasum -a 256`.
const NOTE: &[u8] = b"a note\n";
const NOTE_SHA256: &str = "037279912cb60d7be67228853b057cc642443b4ce29b8a5a5bfbb68234b0b962";

fn reference(
    owner: &str,
    hex: &str,
    length_bytes: u64,
) -> Result<BlobRef, Box<dyn std::error::Error>> {
    Ok(BlobRef {
        owner_service: BlobOwner::parse(owner)?,
        digest: ContentDigest {
            algorithm: DigestAlgorithm::Sha256,
            hex: DigestHex::parse(hex)?,
        },
        media_type: MediaType::parse("text/markdown")?,
        length_bytes,
    })
}

/// Writes `bytes` where `ratatoskr-notes` stores the blob of `hex`.
fn store(root: &Path, hex: &str, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let (prefix, rest) = hex
        .split_at_checked(2)
        .ok_or("a digest has 64 hex digits")?;
    let directory = root.join("sha256").join(prefix);
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join(rest), bytes)?;
    Ok(())
}

#[tokio::test]
async fn notes_blob_ref_is_verified_before_use() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let reader = NotesBlobReader::new(root.path());
    let note = reference("ratatoskr-notes", NOTE_SHA256, 7)?;

    store(root.path(), NOTE_SHA256, NOTE)?;
    assert_eq!(
        reader.read(&note).await.map_err(|error| error.class())?,
        NOTE,
        "bytes that match the reference are returned"
    );

    // The same length, one byte changed: only a fresh hash finds it.
    store(root.path(), NOTE_SHA256, b"a nore\n")?;
    let read = reader.read(&note).await;
    assert_eq!(
        read.as_ref().map_err(NotesBlobError::class).err(),
        Some("blob_digest"),
        "bytes that do not hash to the digest of the reference fail the build: {read:?}"
    );
    Ok(())
}

#[tokio::test]
async fn foreign_owner_ref_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    // The root does not exist. A reader that touched it before it checked the owner would fail
    // with another class, so `blob_owner` shows that nothing was read.
    let parent = tempfile::tempdir()?;
    let reader = NotesBlobReader::new(parent.path().join("no-such-root"));
    let foreign = reference("ratatoskr-telegram", NOTE_SHA256, 7)?;

    let read = reader.read(&foreign).await;
    assert_eq!(
        read.as_ref().map_err(NotesBlobError::class).err(),
        Some("blob_owner"),
        "a reference of another owner is refused and reads nothing: {read:?}"
    );
    Ok(())
}
