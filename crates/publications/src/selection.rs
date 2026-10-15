//! Which Markdown files of one vault connection a publication selects.

use std::collections::BTreeSet;

use ratatoskr_sites_contracts::{PublicationId, SiteId};

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
#[must_use]
pub fn select(publication: &Publication, files: &[IndexedFile]) -> BTreeSet<String> {
    let _ = (publication, files);
    BTreeSet::new()
}
