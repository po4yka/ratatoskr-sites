//! The streaming receipt of a bundle archive (workspace spec `site-bundles`).
//!
//! The archive may hold its entries in any order, so the manifest can come after the files. The
//! receipt checks the path rules of each entry before it writes a byte of it, hashes each file
//! while it writes it into a staging directory of the site, and checks the manifest when the
//! archive ends. Only a bundle that passed every check is renamed to `bundles/<commit>`; on any
//! refusal the staging directory is removed, so no file of the bundle remains.

use std::collections::BTreeMap;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use ratatoskr_sites_contracts::{BundleManifest, BundlePath, SiteId};
use sha2::{Digest as _, Sha256};
use unicode_normalization::is_nfc;
use uuid::Uuid;

use crate::DeployError;

/// The manifest at the root of an archive and of a stored deploy.
pub(crate) const MANIFEST: &str = "ratatoskr-bundle.json";

/// The directory of the files of a stored deploy.
pub(crate) const FILES: &str = "files";

/// The directory of the deploys of a site.
pub(crate) const BUNDLES: &str = "bundles";

/// The directory of the receipts that run.
const STAGING: &str = ".staging";

/// The largest manifest that the receipt reads.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

/// The size and the SHA-256 of each received file, by path.
type Received = BTreeMap<String, (u64, String)>;

/// A bundle that passed every check, with the directory that holds it.
#[derive(Debug)]
pub(crate) struct Stored {
    /// The manifest of the bundle.
    pub(crate) manifest: BundleManifest,
    /// `true` when this receipt stored the bundle.
    pub(crate) created: bool,
}

/// A staging directory, removed on drop unless it was renamed.
struct Staging {
    path: PathBuf,
    kept: bool,
}

impl Staging {
    fn new(site_dir: &Path) -> Result<Self, DeployError> {
        let path = site_dir
            .join(STAGING)
            .join(Uuid::now_v7().simple().to_string());
        std::fs::create_dir_all(path.join(FILES)).map_err(DeployError::Storage)?;
        Ok(Self { path, kept: false })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Receive the archive `reader` for `site` into `site_dir`, at most `max_bytes` of files.
pub(crate) fn receive(
    site_dir: &Path,
    site: &SiteId,
    max_bytes: u64,
    reader: impl Read,
) -> Result<Stored, DeployError> {
    let mut staging = Staging::new(site_dir)?;
    let (manifest_bytes, received) = unpack(&staging.path, max_bytes, reader)?;
    let manifest = verified(site, &manifest_bytes, &received)?;
    write_new(&staging.path.join(MANIFEST), &manifest_bytes)?;
    let bundles = site_dir.join(BUNDLES);
    std::fs::create_dir_all(&bundles).map_err(DeployError::Storage)?;
    std::fs::rename(&staging.path, bundles.join(manifest.commit.as_str()))
        .map_err(DeployError::Storage)?;
    staging.kept = true;
    Ok(Stored {
        manifest,
        created: true,
    })
}

/// Write each file of the archive into `staging`, and give back the manifest bytes and what was
/// received.
fn unpack(
    staging: &Path,
    max_bytes: u64,
    reader: impl Read,
) -> Result<(Vec<u8>, Received), DeployError> {
    let invalid =
        |error: std::io::Error| DeployError::Invalid(format!("not a tar archive: {error}"));
    let mut archive = tar::Archive::new(reader);
    let mut manifest = None;
    let mut received = Received::new();
    let mut total: u64 = 0;
    for entry in archive.entries().map_err(invalid)? {
        let mut entry = entry.map_err(invalid)?;
        let raw = entry.path_bytes().into_owned();
        let name = String::from_utf8(raw).map_err(|raw| {
            DeployError::UnsafePath(String::from_utf8_lossy(raw.as_bytes()).into())
        })?;
        let name = name.strip_prefix("./").unwrap_or(&name).to_owned();
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                let folder = name.trim_end_matches('/');
                if !folder.is_empty() && folder != "." {
                    checked(folder)?;
                }
                continue;
            }
            tar::EntryType::XGlobalHeader => continue,
            tar::EntryType::Regular | tar::EntryType::Continuous => {}
            _ => return Err(DeployError::UnsafePath(name)),
        }
        if name == MANIFEST {
            if manifest.is_some() {
                return Err(DeployError::UnsafePath(name));
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(MAX_MANIFEST_BYTES)
                .read_to_end(&mut bytes)
                .map_err(invalid)?;
            manifest = Some(bytes);
            continue;
        }
        let path = checked(&name)?;
        if received.contains_key(&path) {
            return Err(DeployError::UnsafePath(path));
        }
        let size = entry.header().size().map_err(invalid)?;
        total = total.saturating_add(size);
        if total > max_bytes {
            return Err(DeployError::TooLarge);
        }
        let file = store(staging, &path, &mut entry)?;
        received.insert(path, file);
    }
    let manifest = manifest.ok_or_else(|| {
        DeployError::Invalid(format!("the archive has no {MANIFEST} at its root"))
    })?;
    Ok((manifest, received))
}

/// The first segment that the gateway reserves for forwarding to Edge.
const FORWARDING_PREFIX: &str = "_r";

/// `path` when it is a confined bundle path in NFC outside the forwarding prefix `_r/`.
pub(crate) fn checked(path: &str) -> Result<String, DeployError> {
    if path.split('/').next() == Some(FORWARDING_PREFIX) {
        return Err(DeployError::ReservedPath(path.to_owned()));
    }
    if !is_nfc(path) || BundlePath::parse(path).is_err() {
        return Err(DeployError::UnsafePath(path.to_owned()));
    }
    Ok(path.to_owned())
}

/// Write the bytes of `entry` to `files/<path>` under `staging`, and give back their length and
/// their SHA-256.
fn store(staging: &Path, path: &str, entry: &mut impl Read) -> Result<(u64, String), DeployError> {
    let target = staging.join(FILES).join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(DeployError::Storage)?;
    }
    let mut file = std::fs::File::create_new(&target).map_err(DeployError::Storage)?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = entry
            .read(&mut buffer)
            .map_err(|error| DeployError::Invalid(format!("the archive ends early: {error}")))?;
        let Some(chunk) = buffer.get(..read) else {
            break;
        };
        if chunk.is_empty() {
            break;
        }
        hasher.update(chunk);
        file.write_all(chunk).map_err(DeployError::Storage)?;
        written = written.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    file.sync_all().map_err(DeployError::Storage)?;
    Ok((written, hex(&hasher.finalize())))
}

/// The manifest of `bytes` when it names `site` and lists exactly the received files with their
/// sizes and digests.
fn verified(
    site: &SiteId,
    bytes: &[u8],
    received: &Received,
) -> Result<BundleManifest, DeployError> {
    let manifest: BundleManifest = serde_json::from_slice(bytes)
        .map_err(|error| DeployError::Invalid(format!("{MANIFEST} is not valid: {error}")))?;
    if &manifest.site_id != site {
        return Err(DeployError::SiteMismatch);
    }
    let listed: BTreeMap<&str, _> = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    if let Some(unlisted) = received
        .keys()
        .find(|path| !listed.contains_key(path.as_str()))
    {
        return Err(DeployError::UnlistedFile(unlisted.clone()));
    }
    for file in &manifest.files {
        let path = checked(file.path.as_str())?;
        let matches = received.get(&path).is_some_and(|(size, sha256)| {
            *size == file.size_bytes && sha256 == file.sha256.as_str()
        });
        if !matches {
            return Err(DeployError::DigestMismatch(path));
        }
    }
    Ok(manifest)
}

/// Write `bytes` to a new file at `path`.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), DeployError> {
    let mut file = std::fs::File::create_new(path).map_err(DeployError::Storage)?;
    file.write_all(bytes).map_err(DeployError::Storage)?;
    file.sync_all().map_err(DeployError::Storage)
}

/// Lower-case hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
