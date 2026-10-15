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
/// before it, if any. Then keep at most `retention` deploys ([`retain`]).
pub(crate) fn activate(
    site_dir: &Path,
    commit: &str,
    retention: usize,
) -> Result<Option<String>, DeployError> {
    let target = PathBuf::from(BUNDLES).join(commit);
    if !site_dir.join(&target).join(MANIFEST).is_file() {
        return Err(DeployError::DeployNotFound);
    }
    let before = linked(site_dir, CURRENT);
    if before.as_deref() == Some(commit) {
        return Ok(linked(site_dir, PREVIOUS));
    }
    // When the link was last replaced: the last activation. A deploy stored after it waits for
    // its own activation, so the retention below keeps it.
    let last_activation = std::fs::symlink_metadata(site_dir.join(CURRENT))
        .and_then(|link| link.modified())
        .ok();
    switch(site_dir, CURRENT, &target)?;
    if let Some(before) = &before {
        switch(site_dir, PREVIOUS, &PathBuf::from(BUNDLES).join(before))?;
    }
    // The two renames are durable only when the site directory is on disk.
    if let Err(error) = std::fs::File::open(site_dir).and_then(|dir| dir.sync_all()) {
        tracing::warn!(%error, "the site directory could not be synced");
    }
    retain(site_dir, retention, last_activation)?;
    Ok(before)
}

/// Keep the current deploy, the previous deploy, every deploy stored after `last_activation`, and
/// the newest others up to `retention` deploys in all, by the time each deploy directory was
/// stored, and remove the rest.
fn retain(
    site_dir: &Path,
    retention: usize,
    last_activation: Option<std::time::SystemTime>,
) -> Result<(), DeployError> {
    let linked: Vec<String> = [CURRENT, PREVIOUS]
        .iter()
        .filter_map(|name| linked(site_dir, name))
        .collect();
    let mut others = Vec::new();
    for entry in std::fs::read_dir(site_dir.join(BUNDLES)).map_err(DeployError::Storage)? {
        let entry = entry.map_err(DeployError::Storage)?;
        let commit = entry.file_name().to_string_lossy().into_owned();
        if linked.contains(&commit) {
            continue;
        }
        let stored_at = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map_err(DeployError::Storage)?;
        if last_activation.is_some_and(|activated| stored_at > activated) {
            continue;
        }
        others.push((stored_at, commit, entry.path()));
    }
    // The newest first; one time breaks ties by the commit, so the order is stable.
    others.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let room = retention.saturating_sub(linked.len());
    for (_, commit, path) in others.into_iter().skip(room) {
        tracing::info!(commit, "a deploy beyond the retention count is removed");
        std::fs::remove_dir_all(path).map_err(DeployError::Storage)?;
    }
    Ok(())
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
