//! The gateway projection `site.json` of a site (workspace design D2 of
//! `add-site-gateway-hosting`): the hostnames that the gateway serves for the site and whether the
//! site is approved. The sites service rewrites it from its approval projection on every approval
//! event, by a temporary file and `rename(2)`, so the gateway reads the old file or the new one.

use std::io::Write as _;
use std::path::Path;

use uuid::Uuid;

/// The name of the projection in the directory of a site.
pub const SITE_JSON: &str = "site.json";

/// The content of `site.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteProjection {
    /// The site.
    pub site_id: String,
    /// `true` while the approval of the site is in force.
    pub approved: bool,
    /// The hostnames that select the site; empty for a site that is not approved.
    pub hostnames: Vec<String>,
}

impl SiteProjection {
    /// The projection of `site_id` in the approval state `approved` with the approved `hostname`.
    /// A site that is not approved names no hostname.
    #[must_use]
    pub fn of(site_id: &str, approved: bool, hostname: &str) -> Self {
        Self {
            site_id: site_id.to_owned(),
            approved,
            hostnames: if approved {
                vec![hostname.to_ascii_lowercase()]
            } else {
                Vec::new()
            },
        }
    }
}

/// Write `projection` as `<root>/<site id>/site.json` by a temporary file and `rename(2)`.
///
/// # Errors
///
/// The I/O error when the directory or the file cannot be written.
pub fn write_projection(root: &Path, projection: &SiteProjection) -> std::io::Result<()> {
    let dir = root.join(&projection.site_id);
    std::fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec_pretty(projection).map_err(std::io::Error::other)?;
    let temporary = dir.join(format!(".{SITE_JSON}-{}", Uuid::now_v7().simple()));
    let written = std::fs::File::create_new(&temporary).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_all()
    });
    let renamed = written.and_then(|()| std::fs::rename(&temporary, dir.join(SITE_JSON)));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    renamed
}
