//! The deploys of the static bundles of a site (workspace change `add-site-gateway-hosting`,
//! SITE-026, specs `site-bundles` and `site-gateway`).
//!
//! The sites service is the only writer of the sites tree. For each site it holds:
//!
//! ```text
//! <root>/<site id>/
//! ├── site.json                 the gateway projection, written by the sites service
//! ├── bundles/<commit>/         one verified deploy, read only after it is written
//! │   ├── ratatoskr-bundle.json the bundle manifest
//! │   └── files/<path>          the files that the manifest lists
//! ├── current  -> bundles/<c2>  replaced by rename(2)
//! └── previous -> bundles/<c1>  replaced by rename(2)
//! ```

mod receipt;

use std::path::PathBuf;

use ratatoskr_sites_contracts::{DeployStored, SiteId};
use ratatoskr_sites_persistence::Database;
use tokio_util::io::SyncIoBridge;

/// Why a deploy or an activation was refused or failed. Each variant has a stable code
/// ([`DeployError::code`]) and an HTTP status ([`DeployError::status`]).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeployError {
    /// The site has no approved manifest.
    #[error("the site does not exist")]
    SiteNotFound,
    /// The archive holds a file that the manifest does not list.
    #[error("the bundle holds the unlisted file {0}")]
    UnlistedFile(String),
    /// A listed file is absent, or its bytes do not match its size and digest.
    #[error("the file {0} does not match the bundle manifest")]
    DigestMismatch(String),
    /// A path is not a confined bundle path, or an entry is a link or a device.
    #[error("the bundle path {0} is not safe")]
    UnsafePath(String),
    /// A path is under `_r/`, which the gateway reserves for forwarding.
    #[error("the bundle path {0} is reserved")]
    ReservedPath(String),
    /// The manifest names another site than the route.
    #[error("the bundle is for another site")]
    SiteMismatch,
    /// The bundle is larger than the configured limit.
    #[error("the bundle is larger than the limit")]
    TooLarge,
    /// The body is not a tar archive with a valid `ratatoskr-bundle.json`.
    #[error("the bundle is not valid: {0}")]
    Invalid(String),
    /// No deploy of the commit is stored for the site.
    #[error("no deploy of the commit is stored")]
    DeployNotFound,
    /// The store could not be written or read.
    #[error("the deploy store failed")]
    Storage(#[source] std::io::Error),
    /// A query failed.
    #[error("a query failed")]
    Database(#[from] sqlx::Error),
}

impl DeployError {
    /// The stable code of the error envelope.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SiteNotFound => "sites.site_not_found",
            Self::UnlistedFile(_) => "sites.bundle.unlisted_file",
            Self::DigestMismatch(_) => "sites.bundle.digest_mismatch",
            Self::UnsafePath(_) => "sites.bundle.unsafe_path",
            Self::ReservedPath(_) => "sites.bundle.reserved_path",
            Self::SiteMismatch => "sites.bundle.site_mismatch",
            Self::TooLarge => "sites.bundle.too_large",
            Self::Invalid(_) => "sites.request.invalid",
            Self::DeployNotFound => "sites.bundle.not_found",
            Self::Storage(_) | Self::Database(_) => "sites.service_unavailable",
        }
    }

    /// The HTTP status of the answer.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::SiteNotFound | Self::DeployNotFound => 404,
            Self::UnlistedFile(_)
            | Self::DigestMismatch(_)
            | Self::UnsafePath(_)
            | Self::ReservedPath(_)
            | Self::SiteMismatch => 422,
            Self::TooLarge => 413,
            Self::Invalid(_) => 400,
            Self::Storage(_) | Self::Database(_) => 503,
        }
    }
}

/// The bounds of the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest bundle, the sum of its file sizes.
    pub max_bundle_bytes: u64,
    /// How many deploys a site keeps, the current and the previous one among them.
    pub retention: usize,
}

/// The deploy store under the sites tree `root`.
#[derive(Debug, Clone)]
pub struct DeployStore {
    #[expect(dead_code, reason = "the approval check of task 4.6 reads it")]
    database: Database,
    root: PathBuf,
    limits: Limits,
}

impl DeployStore {
    /// A store under `root` that reads the approval projection of `database`.
    #[must_use]
    pub const fn new(database: Database, root: PathBuf, limits: Limits) -> Self {
        Self {
            database,
            root,
            limits,
        }
    }

    /// The directory of `site` in the sites tree.
    #[must_use]
    pub fn site_dir(&self, site: &SiteId) -> PathBuf {
        self.root.join(site.as_str())
    }

    /// Receive the bundle archive `body` for `site`: stream it into a staging directory with every
    /// check of the receipt, and store it as `bundles/<commit>` only when every check passed.
    ///
    /// # Errors
    ///
    /// [`DeployError`] when the bundle is refused or cannot be stored.
    pub async fn receive<R>(&self, site: &SiteId, body: R) -> Result<DeployStored, DeployError>
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
    {
        let site_dir = self.site_dir(site);
        let owned = site.clone();
        let max_bytes = self.limits.max_bundle_bytes;
        let reader = SyncIoBridge::new(body);
        let stored = tokio::task::spawn_blocking(move || {
            receipt::receive(&site_dir, &owned, max_bytes, reader)
        })
        .await
        .map_err(|_| DeployError::Storage(std::io::Error::other("the receipt stopped")))??;
        answer(stored)
    }
}

/// The answer of a receipt that stored or found `stored`.
fn answer(stored: receipt::Stored) -> Result<DeployStored, DeployError> {
    let manifest = stored.manifest;
    let file_count = u32::try_from(manifest.files.len())
        .map_err(|_| DeployError::Invalid("the bundle lists too many files".to_owned()))?;
    let size_bytes = manifest
        .files
        .iter()
        .fold(0_u64, |total, file| total.saturating_add(file.size_bytes));
    Ok(DeployStored {
        site_id: manifest.site_id,
        commit: manifest.commit,
        file_count,
        size_bytes,
        created: stored.created,
    })
}
