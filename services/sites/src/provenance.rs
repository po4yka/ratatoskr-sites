//! Which publications put each page and each asset into a release.
//!
//! A note can be selected by more than one publication of a site. The delivery routes serve a page,
//! an asset or a search hit only while one of its publications is in the effective set, so a
//! publication that the owner removes leaves the delivery at once, before the next build
//! (workspace spec `site-manifest`, "a removal stops at once").

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_sites_publications::{BuildError, IndexedFile, assign_slugs, select};
use ratatoskr_sites_release::{Built, SiteInput, selected_notes};
use sqlx::PgConnection;
use uuid::Uuid;

/// The vault paths of the notes of a release and the publications of its pages.
#[derive(Debug, Default)]
pub(crate) struct Provenance {
    /// The indexed file of each published note and attachment, by vault connection.
    pub(crate) sources: Vec<(Uuid, IndexedFile)>,
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
            let chosen = select(&input.publication, input.files);
            for file in input
                .files
                .iter()
                .filter(|file| chosen.contains(&file.path))
            {
                by_path
                    .entry(file.path.clone())
                    .or_default()
                    .insert(publication_id.clone());
                let known = sources
                    .iter()
                    .any(|(connection, source): &(Uuid, IndexedFile)| {
                        connection == connection_id && source.path == file.path
                    });
                if !known {
                    sources.push((*connection_id, file.clone()));
                }
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

    /// Add to the sources the attachments whose bytes `built` publishes as assets.
    pub(crate) fn add_attachments(
        &mut self,
        publications: &[(Uuid, String)],
        site: &SiteInput<'_>,
        built: &Built,
    ) {
        let assets: BTreeSet<&str> = built
            .assets
            .iter()
            .map(|asset| asset.digest.hex.as_str())
            .collect();
        for ((connection_id, _), input) in publications.iter().zip(&site.publications) {
            for file in input.files.iter().filter(|file| {
                !file.is_markdown() && assets.contains(file.revision.digest.hex.as_str())
            }) {
                let known = self.sources.iter().any(|(connection, source)| {
                    connection == connection_id && source.path == file.path
                });
                if !known {
                    self.sources.push((*connection_id, file.clone()));
                }
            }
        }
    }

    /// Write the sources of the release `release_id`: each published note and attachment, with
    /// the indexed file that the build read.
    ///
    /// # Errors
    ///
    /// The query error of a row that cannot be written.
    pub(crate) async fn write(
        &self,
        connection: &mut PgConnection,
        release_id: &str,
    ) -> Result<(), sqlx::Error> {
        for (connection_id, file) in &self.sources {
            sqlx::query(
                "insert into sites.release_sources (release_id, connection_id, path, file) \
                 values ($1, $2, $3, $4) on conflict do nothing",
            )
            .bind(release_id)
            .bind(connection_id)
            .bind(&file.path)
            .bind(sqlx::types::Json(file))
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
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
