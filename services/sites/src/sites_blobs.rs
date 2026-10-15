//! The sites blob root: the pages and assets of each release, by SHA-256
//! (`/mnt/nvme/ratatoskr/blobs/ratatoskr-sites/sha256/<2 hex>/<rest>`).

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// The content-addressed store of this service.
#[derive(Debug, Clone)]
pub struct SitesBlobs {
    root: PathBuf,
}

impl SitesBlobs {
    /// The store at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The path of the blob `hex`.
    #[must_use]
    pub fn path(&self, hex: &str) -> Option<PathBuf> {
        let (prefix, rest) = hex.split_at_checked(2)?;
        Some(self.root.join("sha256").join(prefix).join(rest))
    }

    /// Store `bytes` under `hex`, their SHA-256, unless the blob is there already. The bytes go to
    /// a staging file first and are renamed into place, so a reader never sees a part of a blob.
    ///
    /// # Errors
    ///
    /// The I/O error of a failed write.
    pub async fn store(&self, hex: &str, bytes: &[u8]) -> std::io::Result<()> {
        let target = self
            .path(hex)
            .ok_or_else(|| std::io::Error::other("a digest has 64 hex digits"))?;
        if tokio::fs::try_exists(&target).await? {
            return Ok(());
        }
        let staging = self.root.join("staging");
        tokio::fs::create_dir_all(&staging).await?;
        let part = staging.join(format!("{}.part", Uuid::now_v7()));
        tokio::fs::write(&part, bytes).await?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let renamed = tokio::fs::rename(&part, &target).await;
        if renamed.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        renamed
    }

    /// The root of the store.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}
