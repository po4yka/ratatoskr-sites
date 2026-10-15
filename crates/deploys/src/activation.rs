//! The activation of a stored deploy (workspace spec `site-bundles`, "Activation switches the
//! current deploy atomically").
//!
//! `current` and `previous` are symbolic links in the site directory to `bundles/<commit>`. Each is
//! replaced by a new link under a temporary name and `rename(2)`, so a reader that resolves
//! `current` finds the old deploy or the new one, never a missing link or a mix.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::DeployError;
use crate::receipt::{BUNDLES, MANIFEST};

/// The link to the deploy that the gateway serves.
pub(crate) const CURRENT: &str = "current";

/// The link to the deploy that was current before.
pub(crate) const PREVIOUS: &str = "previous";

/// Make the deploy of `commit` current in `site_dir`, and give back the commit that was current
/// before it, if any.
pub(crate) fn activate(site_dir: &Path, commit: &str) -> Result<Option<String>, DeployError> {
    let target = PathBuf::from(BUNDLES).join(commit);
    if !site_dir.join(&target).join(MANIFEST).is_file() {
        return Err(DeployError::DeployNotFound);
    }
    let before = linked(site_dir, CURRENT);
    if before.as_deref() == Some(commit) {
        return Ok(linked(site_dir, PREVIOUS));
    }
    switch(site_dir, CURRENT, &target)?;
    if let Some(before) = &before {
        switch(site_dir, PREVIOUS, &PathBuf::from(BUNDLES).join(before))?;
    }
    Ok(before)
}

/// The commit that the link `name` of `site_dir` names, if the link exists.
pub(crate) fn linked(site_dir: &Path, name: &str) -> Option<String> {
    let target = std::fs::read_link(site_dir.join(name)).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// Point the link `name` of `site_dir` at `target` by a temporary link and `rename(2)`.
fn switch(site_dir: &Path, name: &str, target: &Path) -> Result<(), DeployError> {
    let temporary = site_dir.join(format!(".{name}-{}", Uuid::now_v7().simple()));
    std::os::unix::fs::symlink(target, &temporary).map_err(DeployError::Storage)?;
    std::fs::rename(&temporary, site_dir.join(name)).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        DeployError::Storage(error)
    })
}
