//! Bundles for the receipt and activation tests: a manifest and a tar archive built in memory, and
//! a store over a disposable database and a temporary sites tree.

#![allow(dead_code, reason = "each test file uses a part of the support")]

use std::path::Path;

use ratatoskr_sites_contracts::SiteId;
use ratatoskr_sites_deploys::{DeployStore, Limits};
use ratatoskr_sites_persistence::test_support::TestDatabase;
use sha2::{Digest as _, Sha256};

pub(crate) type Failure = Box<dyn std::error::Error>;

/// The site of the store.
pub(crate) const SITE: &str = "site-a";

/// A commit id of 40 hex digits that ends with `tail`.
pub(crate) fn commit(tail: char) -> String {
    format!("{}{tail}", "c".repeat(39))
}

/// The lower-case hex SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// One bundle: its site, its commit and its files, each with its bytes and its `hashed` flag.
#[derive(Debug, Clone)]
pub(crate) struct Bundle {
    pub(crate) site: String,
    pub(crate) commit: String,
    pub(crate) files: Vec<(String, Vec<u8>, bool)>,
    pub(crate) entry_fallback: bool,
}

impl Bundle {
    /// A bundle of `SITE` at `commit` with `index.html` and a hashed `assets/app.3f9a1c.js`.
    pub(crate) fn new(commit: &str) -> Self {
        Self {
            site: SITE.to_owned(),
            commit: commit.to_owned(),
            files: vec![
                (
                    "assets/app.3f9a1c.js".to_owned(),
                    format!("console.log('{commit}');").into_bytes(),
                    true,
                ),
                (
                    "index.html".to_owned(),
                    format!("<!doctype html><title>{commit}</title>").into_bytes(),
                    false,
                ),
            ],
            entry_fallback: true,
        }
    }

    /// The same bundle with the file `path`.
    pub(crate) fn with(mut self, path: &str, bytes: &[u8], hashed: bool) -> Self {
        self.files.push((path.to_owned(), bytes.to_vec(), hashed));
        self.files.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    /// `ratatoskr-bundle.json` of the bundle.
    pub(crate) fn manifest(&self) -> Vec<u8> {
        let files: Vec<serde_json::Value> = self
            .files
            .iter()
            .map(|(path, bytes, hashed)| {
                serde_json::json!({
                    "path": path,
                    "size_bytes": bytes.len(),
                    "sha256": sha256(bytes),
                    "hashed": hashed,
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "site_id": self.site,
            "repository": "owner/docs-site",
            "commit": self.commit,
            "entry_document": "index.html",
            "entry_fallback": self.entry_fallback,
            "files": files,
        }))
        .unwrap_or_default()
    }

    /// The archive: the manifest, then each file as the manifest lists it.
    pub(crate) fn archive(&self) -> Result<Vec<u8>, Failure> {
        self.archive_with(&self.files, |_| Ok(()))
    }

    /// The archive with `files` in place of the listed bytes, and the entries that `extra` appends.
    pub(crate) fn archive_with(
        &self,
        files: &[(String, Vec<u8>, bool)],
        extra: impl FnOnce(&mut tar::Builder<Vec<u8>>) -> Result<(), Failure>,
    ) -> Result<Vec<u8>, Failure> {
        let mut builder = tar::Builder::new(Vec::new());
        append(&mut builder, "ratatoskr-bundle.json", &self.manifest())?;
        for (path, bytes, _) in files {
            append(&mut builder, path, bytes)?;
        }
        extra(&mut builder)?;
        Ok(builder.into_inner()?)
    }
}

/// Append a regular file.
pub(crate) fn append(
    builder: &mut tar::Builder<Vec<u8>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), Failure> {
    let mut header = tar::Header::new_gnu();
    header.set_size(u64::try_from(bytes.len())?);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    builder.append_data(&mut header, path, bytes)?;
    Ok(())
}

/// Append an entry whose name is `name` as it is, without the checks of `tar::Builder`.
pub(crate) fn append_raw(
    builder: &mut tar::Builder<Vec<u8>>,
    name: &[u8],
    kind: tar::EntryType,
    bytes: &[u8],
    link: Option<&str>,
) -> Result<(), Failure> {
    let mut header = tar::Header::new_old();
    for (slot, byte) in header.as_old_mut().name.iter_mut().zip(name) {
        *slot = *byte;
    }
    header.set_size(u64::try_from(bytes.len())?);
    header.set_mode(0o644);
    header.set_entry_type(kind);
    if let Some(link) = link {
        header.set_link_name(link)?;
    }
    header.set_cksum();
    builder.append(&header, bytes)?;
    Ok(())
}

/// The store of one test.
pub(crate) struct Store {
    pub(crate) database: TestDatabase,
    pub(crate) root: tempfile::TempDir,
    pub(crate) deploys: DeployStore,
}

impl Store {
    /// A store with a retention of `retention` and `SITE` approved.
    pub(crate) async fn new(retention: usize) -> Result<Self, Failure> {
        Self::with_limit(retention, 64 * 1024 * 1024).await
    }

    /// A store with a retention of `retention`, a bundle limit of `max_bundle_bytes`, and `SITE`
    /// approved.
    pub(crate) async fn with_limit(
        retention: usize,
        max_bundle_bytes: u64,
    ) -> Result<Self, Failure> {
        let database = TestDatabase::create().await?;
        let root = tempfile::tempdir()?;
        let deploys = DeployStore::new(
            database.database.clone(),
            root.path().to_path_buf(),
            Limits {
                max_bundle_bytes,
                retention,
            },
        );
        let fixture = Self {
            database,
            root,
            deploys,
        };
        fixture.approval(SITE, "approved").await?;
        Ok(fixture)
    }

    /// Give `site` the approval state `state` in the approval projection.
    pub(crate) async fn approval(&self, site: &str, state: &str) -> Result<(), Failure> {
        sqlx::query(
            "insert into sites.approval_projection (site_id, approved_digest, state, \
             capabilities, hostname, changed_at) values ($1, $2, $3, '{}', 'a.example.test', \
             '-infinity') on conflict (site_id) do update set state = excluded.state",
        )
        .bind(site)
        .bind("0".repeat(64))
        .bind(state)
        .execute(self.database.pool())
        .await?;
        Ok(())
    }

    /// The site of the store.
    pub(crate) fn site() -> Result<SiteId, Failure> {
        Ok(SiteId::parse(SITE)?)
    }

    /// Every file and link under the directory of `SITE`.
    pub(crate) fn stored(&self) -> Result<Vec<String>, Failure> {
        let mut found = Vec::new();
        let dir = self.deploys.site_dir(&Self::site()?);
        if dir.exists() {
            walk(&dir, &dir, &mut found)?;
        }
        found.sort();
        Ok(found)
    }

    pub(crate) async fn cleanup(self) -> Result<(), Failure> {
        Ok(self.database.cleanup().await?)
    }
}

fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) -> Result<(), Failure> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            walk(base, &path, found)?;
        } else {
            found.push(path.strip_prefix(base)?.display().to_string());
        }
    }
    Ok(())
}

/// Append `bytes` as the regular file `path` whose ustar size field says 0, with a PAX `size`
/// record before it that gives the real length, as the review probe of M2 does.
pub(crate) fn append_pax_sized(
    builder: &mut tar::Builder<Vec<u8>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), Failure> {
    let digits = bytes.len().to_string();
    // A PAX record is "<length> size=<digits>\n", and its length counts its own digits.
    let mut length = digits.len() + 7;
    length += length.to_string().len();
    let record = format!("{length} size={digits}\n");
    let mut extension = tar::Header::new_ustar();
    extension.set_path("PaxHeader")?;
    extension.set_entry_type(tar::EntryType::XHeader);
    extension.set_size(u64::try_from(record.len())?);
    extension.set_mode(0o644);
    extension.set_cksum();
    let mut file = tar::Header::new_ustar();
    file.set_path(path)?;
    file.set_entry_type(tar::EntryType::Regular);
    file.set_size(0);
    file.set_mode(0o644);
    file.set_cksum();
    let out = builder.get_mut();
    out.extend_from_slice(extension.as_bytes());
    out.extend_from_slice(&padded(record.as_bytes()));
    out.extend_from_slice(file.as_bytes());
    out.extend_from_slice(&padded(bytes));
    Ok(())
}

/// `bytes` padded with zeros to a whole number of 512-byte blocks.
fn padded(bytes: &[u8]) -> Vec<u8> {
    let mut block = bytes.to_vec();
    block.resize(bytes.len().div_ceil(512) * 512, 0);
    block
}
