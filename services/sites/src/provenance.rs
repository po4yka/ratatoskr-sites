//! Which publications put each page and each asset into a release.
//!
//! A note can be selected by more than one publication of a site. The delivery routes serve a page,
//! an asset or a search hit only while one of its publications is in the effective set, so a
//! publication that the owner removes leaves the delivery at once, before the next build
//! (workspace spec `site-manifest`, "a removal stops at once").

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_sites_publications::{BuildError, assign_slugs, select};
use ratatoskr_sites_release::{Built, SiteInput, selected_notes};
use uuid::Uuid;

/// The vault paths of the notes of a release and the publications of its pages.
#[derive(Debug, Default)]
pub(crate) struct Provenance {
    /// The vault path of each published note, by vault connection.
    pub(crate) sources: Vec<(Uuid, String)>,
    /// The publications of each page, by slug, in ascending order.
    pages: BTreeMap<String, Vec<String>>,
}

impl Provenance {
    /// The provenance of the release of `site`. `publications` names the vault connection and the
    /// id of each publication of `site`, in the same order.
    ///
    /// # Errors
    ///
    /// The [`BuildError`] of a slug collision or an invalid slug.
    pub(crate) fn of(
        publications: &[(Uuid, String)],
        site: &SiteInput<'_>,
    ) -> Result<Self, BuildError> {
        let mut sources = Vec::new();
        let mut by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ((connection_id, publication_id), input) in publications.iter().zip(&site.publications)
        {
            for path in select(&input.publication, input.files) {
                by_path
                    .entry(path.clone())
                    .or_default()
                    .insert(publication_id.clone());
                sources.push((*connection_id, path));
            }
        }
        let pages = assign_slugs(&selected_notes(site))?
            .into_iter()
            .map(|(slug, path)| {
                let publications = by_path.get(&path).cloned().unwrap_or_default();
                (slug.as_str().to_owned(), publications.into_iter().collect())
            })
            .collect();
        Ok(Self { sources, pages })
    }

    /// The publications of the page `slug`.
    pub(crate) fn page(&self, slug: &str) -> &[String] {
        self.pages.get(slug).map_or(&[], Vec::as_slice)
    }

    /// The publications of each asset of `built`: those of the pages that use it.
    pub(crate) fn assets(&self, built: &Built) -> BTreeMap<String, Vec<String>> {
        let mut assets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for page in &built.pages {
            for asset in &page.page.assets {
                assets
                    .entry(asset.sha256.as_str().to_owned())
                    .or_default()
                    .extend(self.page(page.page.slug.as_str()).iter().cloned());
            }
        }
        assets
            .into_iter()
            .map(|(digest, publications)| (digest, publications.into_iter().collect()))
            .collect()
    }
}
