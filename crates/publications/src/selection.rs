//! Which Markdown files of one vault connection a publication selects.

use std::collections::BTreeSet;

use ratatoskr_sites_contracts::{PublicationId, SiteId};
use serde_json::Value;

use crate::index::IndexedFile;

/// Which files a publication takes when no other rule decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationMode {
    /// Every Markdown file in the included folders.
    All,
    /// Only a file with `publish: true`, a `publish` list that names the site, or an included tag.
    Flagged,
}

/// A publication: a filter over one vault connection, for one site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    /// The site that the publication belongs to.
    pub site_id: SiteId,
    /// The publication, unique in its site.
    pub publication_id: PublicationId,
    /// The rule when no other rule decides.
    pub mode: PublicationMode,
    /// The folders that the publication reads, as vault paths without a trailing `/`. Empty: the
    /// whole vault.
    pub included_folders: Vec<String>,
    /// The folders that the publication never reads.
    pub excluded_folders: Vec<String>,
    /// The tags that select a file, without `#`.
    pub included_tags: Vec<String>,
}

/// The vault paths of the Markdown files of `files` that `publication` selects.
///
/// The precedence of the workspace spec `site-publications`, highest first: a file that is not
/// Markdown, or that is under `.obsidian/`, is never selected; an excluded folder excludes; a file
/// outside the included folders (when any are named) is excluded; `publish: false` excludes; a
/// `publish` list that does not name the site excludes; `publish: true` or a list that names the
/// site includes; an included tag includes; then the mode `all` includes and the mode `flagged`
/// excludes. A `publish` value of another type excludes, so that an unknown form never publishes.
#[must_use]
pub fn select(publication: &Publication, files: &[IndexedFile]) -> BTreeSet<String> {
    files
        .iter()
        .filter(|file| decide(publication, file))
        .map(|file| file.path.clone())
        .collect()
}

fn decide(publication: &Publication, file: &IndexedFile) -> bool {
    if !file.is_markdown() || in_folder(&file.path, ".obsidian") {
        return false;
    }
    if publication
        .excluded_folders
        .iter()
        .any(|folder| in_folder(&file.path, folder))
    {
        return false;
    }
    if !publication.included_folders.is_empty()
        && !publication
            .included_folders
            .iter()
            .any(|folder| in_folder(&file.path, folder))
    {
        return false;
    }
    match flag(publication, file) {
        Flag::Excludes => return false,
        Flag::Includes => return true,
        Flag::Absent => {}
    }
    if file
        .tags
        .iter()
        .any(|tag| has_included_tag(publication, tag))
    {
        return true;
    }
    publication.mode == PublicationMode::All
}

/// What the frontmatter key `publish` says about one site.
enum Flag {
    Includes,
    Excludes,
    Absent,
}

fn flag(publication: &Publication, file: &IndexedFile) -> Flag {
    let site = publication.site_id.as_str();
    match file.frontmatter_value("publish") {
        None | Some(Value::Null) => Flag::Absent,
        Some(Value::Bool(true)) => Flag::Includes,
        Some(Value::String(named)) if named == site => Flag::Includes,
        Some(Value::Array(sites)) if sites.iter().any(|named| named.as_str() == Some(site)) => {
            Flag::Includes
        }
        Some(_) => Flag::Excludes,
    }
}

/// Whether `path` is in `folder` or one of its subfolders. A trailing `/` of the folder is ignored.
fn in_folder(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    !folder.is_empty()
        && path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether the note tag `tag` (with its `#`) is an included tag of the publication. Tags compare
/// without case, as in Obsidian, and a nested tag `#public/sub` is not the tag `public`.
fn has_included_tag(publication: &Publication, tag: &str) -> bool {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    publication
        .included_tags
        .iter()
        .any(|included| included.trim_start_matches('#').eq_ignore_ascii_case(tag))
}
