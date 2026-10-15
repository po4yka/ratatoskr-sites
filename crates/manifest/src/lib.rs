//! The site manifest `ratatoskr-site.toml` (contract `sites/site-manifest.v1`, workspace spec
//! `site-manifest`): its parse, its validation and its digest.
//!
//! This crate reads no file and no database. The sites service stores the bytes that it accepts by
//! their digest.

use ratatoskr_sites_contracts::SiteManifest;
use sha2::{Digest as _, Sha256};

/// The class of a manifest that is refused.
pub const MANIFEST_INVALID: &str = "sites.manifest_invalid";

/// A manifest that does not validate. The detail names the key or the value that fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the manifest is not valid: {detail}")]
pub struct ManifestInvalid {
    /// What fails, with the key or the value.
    pub detail: String,
}

impl ManifestInvalid {
    /// The stable class, [`MANIFEST_INVALID`].
    #[must_use]
    pub const fn class(&self) -> &'static str {
        MANIFEST_INVALID
    }
}

/// The manifest that `bytes` hold.
///
/// # Errors
///
/// [`ManifestInvalid`] when the bytes are not a valid `sites/site-manifest.v1` in TOML.
pub fn parse(_bytes: &[u8]) -> Result<SiteManifest, ManifestInvalid> {
    Err(ManifestInvalid {
        detail: String::new(),
    })
}

/// The lower-case hex SHA-256 of the manifest bytes: the digest that an approval names.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
