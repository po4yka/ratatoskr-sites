//! The loader of a fixture vault. Enabled by the `test-support` feature, so it is never compiled
//! into a service binary.
//!
//! A fixture vault is a directory with `vault/`, the files of one vault connection, and
//! `index.json`, what the notes internal index route gives for each file without its revision.
//! The loader computes the revision `BlobRef` from the bytes in `vault/`.

use std::fmt::Write as _;
use std::path::Path;

use ratatoskr_identifiers::{
    BlobOwner, BlobRef, ContentDigest, DigestAlgorithm, DigestHex, MediaType,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::index::IndexedFile;

/// A fixture could not be loaded.
#[derive(Debug, thiserror::Error)]
#[error("the fixture vault could not be loaded: {0}")]
pub struct FixtureError(String);

/// The lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The revision of `bytes` as `ratatoskr-notes` names it.
///
/// # Errors
///
/// [`FixtureError`] when the media type is not valid.
pub fn revision_of(bytes: &[u8], media_type: &str) -> Result<BlobRef, FixtureError> {
    let invalid = |what: &str| FixtureError(format!("invalid {what}"));
    Ok(BlobRef {
        owner_service: BlobOwner::parse("ratatoskr-notes").map_err(|_| invalid("owner"))?,
        digest: ContentDigest {
            algorithm: DigestAlgorithm::Sha256,
            hex: DigestHex::parse(&sha256_hex(bytes)).map_err(|_| invalid("digest"))?,
        },
        media_type: MediaType::parse(media_type).map_err(|_| invalid("media type"))?,
        length_bytes: u64::try_from(bytes.len()).map_err(|_| invalid("length"))?,
    })
}

/// The index of the fixture vault in `directory`.
///
/// # Errors
///
/// [`FixtureError`] when `index.json` or a file of `vault/` cannot be read.
pub fn load_index(directory: &Path) -> Result<Vec<IndexedFile>, FixtureError> {
    let text = std::fs::read_to_string(directory.join("index.json"))
        .map_err(|error| FixtureError(format!("index.json: {error}")))?;
    let entries: Vec<Value> = serde_json::from_str(&text)
        .map_err(|error| FixtureError(format!("index.json: {error}")))?;
    let mut files = Vec::with_capacity(entries.len());
    for mut entry in entries {
        let object = entry
            .as_object_mut()
            .ok_or_else(|| FixtureError("an entry is not an object".to_owned()))?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| FixtureError("an entry has no path".to_owned()))?
            .to_owned();
        let media_type = object
            .remove("media_type")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| FixtureError(format!("{path} has no media_type")))?;
        let bytes = std::fs::read(directory.join("vault").join(&path))
            .map_err(|error| FixtureError(format!("{path}: {error}")))?;
        let revision = serde_json::to_value(revision_of(&bytes, &media_type)?)
            .map_err(|error| FixtureError(format!("{path}: {error}")))?;
        object.insert("revision".to_owned(), revision);
        files.push(
            serde_json::from_value(entry)
                .map_err(|error| FixtureError(format!("{path}: {error}")))?,
        );
    }
    Ok(files)
}
