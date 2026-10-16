//! The streaming receipt of a bundle archive (workspace spec `site-bundles`).
//!
//! The archive may hold its entries in any order, so the manifest can come after the files. The
//! receipt checks the path rules of each entry before it writes a byte of it, hashes each file
//! while it writes it into a staging directory of the site, and checks the manifest when the
//! archive ends. Only a bundle that passed every check is renamed to `bundles/<commit>`; on any
//! refusal the staging directory is removed, so no file of the bundle remains.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
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
        // Keep the site traversable by the gateway, but hide the receipt until publication.
        std::fs::create_dir_all(site_dir.join(STAGING)).map_err(DeployError::Storage)?;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(DeployError::Storage)?;
        let staging = Self { path, kept: false };
        std::fs::create_dir(staging.path.join(FILES)).map_err(DeployError::Storage)?;
        Ok(staging)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Remove every receipt directory under `<root>/<site>/.staging/`, and give back how many.
pub(crate) fn remove_stale(root: &Path) -> Result<usize, DeployError> {
    let sites = match std::fs::read_dir(root) {
        Ok(sites) => sites,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(DeployError::Storage(error)),
    };
    let mut removed = remove_verification(root)?;
    for site in sites {
        let staging = site.map_err(DeployError::Storage)?.path().join(STAGING);
        let Ok(receipts) = std::fs::read_dir(&staging) else {
            continue;
        };
        for receipt in receipts {
            let receipt = receipt.map_err(DeployError::Storage)?.path();
            std::fs::remove_dir_all(&receipt).map_err(DeployError::Storage)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Remove private verifier jobs left by an interrupted process before new work starts.
fn remove_verification(root: &Path) -> Result<usize, DeployError> {
    let entries = match std::fs::read_dir(root.join(".verification")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(DeployError::Storage(error)),
    };
    let mut removed = 0;
    for entry in entries {
        std::fs::remove_dir_all(entry.map_err(DeployError::Storage)?.path())
            .map_err(DeployError::Storage)?;
        removed += 1;
    }
    Ok(removed)
}

/// Receive the archive `reader` for `site` into `site_dir`, at most `max_bytes` of files.
pub(crate) fn receive(
    site_dir: &Path,
    site: &SiteId,
    max_bytes: u64,
    reader: impl Read,
) -> Result<Stored, DeployError> {
    publish(site_dir, prepare(site_dir, site, max_bytes, reader)?)
}

/// A checked archive whose files are still private and removed on drop.
pub(crate) struct Prepared {
    staging: Staging,
    pub(crate) manifest: BundleManifest,
}

impl Prepared {
    /// Write server-produced metadata beside the archive manifest, outside public files.
    pub(crate) fn seal(&self, bytes: &[u8]) -> Result<(), DeployError> {
        write_new(&self.staging.path.join("verification.json"), bytes)
    }
}

/// Check bytes without publishing a deploy directory.
pub(crate) fn prepare(
    site_dir: &Path,
    site: &SiteId,
    max_bytes: u64,
    reader: impl Read,
) -> Result<Prepared, DeployError> {
    let staging = Staging::new(site_dir)?;
    let (manifest_bytes, received) = unpack(&staging.path, max_bytes, reader)?;
    let manifest = verified(site, &manifest_bytes, &received)?;
    write_new(&staging.path.join(MANIFEST), &manifest_bytes)?;
    Ok(Prepared { staging, manifest })
}

/// Publish a prepared receipt. The caller serializes its final authority check and this rename.
pub(crate) fn publish(site_dir: &Path, prepared: Prepared) -> Result<Stored, DeployError> {
    let Prepared {
        mut staging,
        manifest,
    } = prepared;
    let bundles = site_dir.join(BUNDLES);
    std::fs::create_dir_all(&bundles).map_err(DeployError::Storage)?;
    let target = bundles.join(manifest.commit.as_str());
    // A deploy is idempotent on the site and the commit: a stored deploy of the commit is the
    // answer, and the staging directory is removed.
    if target.exists() {
        return stored(&target);
    }
    // Keep the inherited bundle group. No future write needs SGID in this immutable directory;
    // chmod with SGID would be refused by the service unit's RestrictSUIDSGID=yes.
    std::fs::set_permissions(&staging.path, std::fs::Permissions::from_mode(0o750))
        .map_err(DeployError::Storage)?;
    if let Err(error) = std::fs::rename(&staging.path, &target) {
        // Another receipt of the same commit renamed its copy first.
        return if target.exists() {
            stored(&target)
        } else {
            Err(DeployError::Storage(error))
        };
    }
    staging.kept = true;
    // The rename is durable only when the directory that holds it is on disk.
    if let Err(error) = std::fs::File::open(&bundles).and_then(|dir| dir.sync_all()) {
        tracing::warn!(%error, "the bundles directory could not be synced");
    }
    Ok(Stored {
        manifest,
        created: true,
    })
}

/// The deploy that is stored at `target`.
pub(crate) fn stored(target: &Path) -> Result<Stored, DeployError> {
    let bytes = std::fs::read(target.join(MANIFEST)).map_err(DeployError::Storage)?;
    let manifest: BundleManifest = serde_json::from_slice(&bytes)
        .map_err(|error| DeployError::Storage(std::io::Error::other(error)))?;
    Ok(Stored {
        manifest,
        created: false,
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
    let mut folders = BTreeSet::new();
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
                    let folder = checked(folder)?;
                    if received.contains_key(&folder) {
                        return Err(DeployError::UnsafePath(folder));
                    }
                    folders.insert(folder);
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
        claim(&path, &received, &mut folders)?;
        // `entry.size()` is the length after a PAX `size` record; the ustar field alone is not.
        // The bytes that `store` writes are counted too, so no header can pass the limit.
        let budget = max_bytes.saturating_sub(total);
        if entry.size() > budget {
            return Err(DeployError::TooLarge);
        }
        let file = store(staging, &path, &mut entry, budget)?;
        total = total.saturating_add(file.0);
        received.insert(path, file);
    }
    let manifest = manifest.ok_or_else(|| {
        DeployError::Invalid(format!("the archive has no {MANIFEST} at its root"))
    })?;
    Ok((manifest, received))
}

/// Refuse `path` when the archive already holds it, holds a file below it, or holds a file at one
/// of its folders; then record its folders. A file and a folder of one name cannot both be written.
fn claim(
    path: &str,
    received: &Received,
    folders: &mut BTreeSet<String>,
) -> Result<(), DeployError> {
    let unsafe_path = || DeployError::UnsafePath(path.to_owned());
    if received.contains_key(path) || folders.contains(path) {
        return Err(unsafe_path());
    }
    let parents: Vec<&str> = path
        .match_indices('/')
        .filter_map(|(at, _)| path.get(..at))
        .collect();
    if parents.iter().any(|parent| received.contains_key(*parent)) {
        return Err(unsafe_path());
    }
    folders.extend(parents.into_iter().map(str::to_owned));
    Ok(())
}

/// The first segment that the gateway reserves for forwarding to Edge.
const FORWARDING_PREFIX: &str = "_r";

/// `path` when it is a confined bundle path in NFC outside the forwarding prefix `_r/`, and not
/// the manifest itself.
pub(crate) fn checked(path: &str) -> Result<String, DeployError> {
    if path.split('/').next() == Some(FORWARDING_PREFIX) || path == MANIFEST {
        return Err(DeployError::ReservedPath(path.to_owned()));
    }
    if !is_nfc(path) || BundlePath::parse(path).is_err() {
        return Err(DeployError::UnsafePath(path.to_owned()));
    }
    Ok(path.to_owned())
}

/// Write the bytes of `entry` to `files/<path>` under `staging`, at most `budget` bytes, and give
/// back their length and their SHA-256.
fn store(
    staging: &Path,
    path: &str,
    entry: &mut impl Read,
    budget: u64,
) -> Result<(u64, String), DeployError> {
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
        if written > budget {
            return Err(DeployError::TooLarge);
        }
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

#[cfg(test)]
mod private_staging_tests {
    use super::Staging;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn an_unpublished_receipt_is_private() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let staging = Staging::new(root.path())?;
        assert_eq!(
            std::fs::metadata(&staging.path)?.permissions().mode() & 0o777,
            0o700
        );
        Ok(())
    }
    #[test]
    fn publication_needs_no_suid_or_sgid_permission() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let site = ratatoskr_sites_contracts::SiteId::parse("site-test")?;
        let contents = b"verified page";
        let commit = "a".repeat(40);
        let manifest = serde_json::to_vec(&serde_json::json!({
            "site_id": site, "repository": "owner/site", "commit": commit,
            "entry_document": "index.html", "entry_fallback": true,
            "files": [{"path": "index.html", "size_bytes": contents.len(),
                "sha256": super::hex(&<sha2::Sha256 as sha2::Digest>::digest(contents)), "hashed": false}],
        }))?;
        let mut archive = tar::Builder::new(Vec::new());
        for (name, bytes) in [
            (super::MANIFEST, manifest.as_slice()),
            ("index.html", contents.as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(bytes.len())?);
            header.set_mode(0o644);
            archive.append_data(&mut header, name, bytes)?;
        }
        let prepared = super::prepare(
            root.path(),
            &site,
            1024,
            std::io::Cursor::new(archive.into_inner()?),
        )?;
        super::publish(root.path(), prepared)?;
        let mode = std::fs::metadata(root.path().join(super::BUNDLES).join(commit))?
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o7777,
            0o750,
            "publication must work with RestrictSUIDSGID=yes"
        );
        Ok(())
    }
}
