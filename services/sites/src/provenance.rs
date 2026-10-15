//! Which publications put each page and each asset into a release.
//!
//! A note can be selected by more than one publication of a site. The delivery routes serve a page,
//! an asset or a search hit only while one of its publications is in the effective set, so a
//! publication that the owner removes leaves the delivery at once, before the next build
//! (workspace spec `site-manifest`, "a removal stops at once").

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_sites_publications::{BuildError, IndexedFile, assign_slugs, select};
use ratatoskr_sites_release::{Built, DEFAULT_FRONTMATTER_ALLOWLIST, SiteInput, selected_notes};
use sqlx::PgConnection;
use uuid::Uuid;

/// One published file of a release and what its projection may keep.
#[derive(Debug, Clone)]
struct Source {
    connection: Uuid,
    file: IndexedFile,
    /// The frontmatter keys that the publications that select the note allow.
    keys: BTreeSet<String>,
    /// The publications that select the note; none for an attachment.
    publications: BTreeSet<String>,
}

/// The published files of a release and the publications of its pages.
#[derive(Debug, Default)]
pub(crate) struct Provenance {
    sources: Vec<Source>,
    /// The publications of each page, by slug, in ascending order.
    pages: BTreeMap<String, Vec<String>>,
    /// The slug of each published note, by vault path.
    slugs: BTreeMap<String, String>,
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
        let mut sources: Vec<Source> = Vec::new();
        let mut by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ((connection_id, publication_id), input) in publications.iter().zip(&site.publications)
        {
            let chosen = select(&input.publication, input.files);
            let keys = DEFAULT_FRONTMATTER_ALLOWLIST
                .iter()
                .map(|&key| key.to_owned())
                .chain(input.frontmatter_allow.iter().cloned());
            for file in input
                .files
                .iter()
                .filter(|file| chosen.contains(&file.path))
            {
                by_path
                    .entry(file.path.clone())
                    .or_default()
                    .insert(publication_id.clone());
                let same = |source: &Source| {
                    source.connection == *connection_id && source.file.path == file.path
                };
                if !sources.iter().any(same) {
                    sources.push(Source {
                        connection: *connection_id,
                        file: file.clone(),
                        keys: BTreeSet::new(),
                        publications: BTreeSet::new(),
                    });
                }
                let Some(source) = sources.iter_mut().find(|source| same(source)) else {
                    continue;
                };
                source.keys.extend(keys.clone());
                source.publications.insert(publication_id.clone());
            }
        }
        let assigned = assign_slugs(&selected_notes(site))?;
        let slugs = assigned
            .iter()
            .map(|(slug, path)| (path.clone(), slug.as_str().to_owned()))
            .collect();
        let pages = assigned
            .into_iter()
            .map(|(slug, path)| {
                let publications = by_path.get(&path).cloned().unwrap_or_default();
                (slug.as_str().to_owned(), publications.into_iter().collect())
            })
            .collect();
        Ok(Self {
            sources,
            pages,
            slugs,
        })
    }

    /// Take from `built` what rendering resolved: each published note keeps only the links and
    /// embeds that its page resolved (a link inside a comment never is), and the attachments are
    /// added that such a link or embed names (the target that the notes resolver found) and whose
    /// bytes are an asset of the release. A copy with the same bytes at another path is never
    /// added.
    pub(crate) fn add_rendered(
        &mut self,
        publications: &[(Uuid, String)],
        site: &SiteInput<'_>,
        built: &Built,
    ) {
        let rendered: BTreeMap<&str, &ratatoskr_sites_release::BuiltPage> = built
            .pages
            .iter()
            .map(|page| (page.page.slug.as_str(), page))
            .collect();
        for source in &mut self.sources {
            let page = self
                .slugs
                .get(&source.file.path)
                .and_then(|slug| rendered.get(slug.as_str()));
            source.file.links = page.map_or_else(Vec::new, |page| page.resolved_links.clone());
            source.file.embeds = page.map_or_else(Vec::new, |page| page.resolved_embeds.clone());
        }
        let assets: BTreeSet<&str> = built
            .assets
            .iter()
            .map(|asset| asset.digest.hex.as_str())
            .collect();
        let targets: BTreeSet<(Uuid, String)> = self
            .sources
            .iter()
            .flat_map(|source| {
                source
                    .file
                    .links
                    .iter()
                    .chain(&source.file.embeds)
                    .filter_map(|link| link.target.clone())
                    .map(|target| (source.connection, target))
            })
            .collect();
        for ((connection_id, _), input) in publications.iter().zip(&site.publications) {
            for file in input.files {
                let used = !file.is_markdown()
                    && assets.contains(file.revision.digest.hex.as_str())
                    && targets.contains(&(*connection_id, file.path.clone()));
                let known = self.sources.iter().any(|source| {
                    source.connection == *connection_id && source.file.path == file.path
                });
                if used && !known {
                    self.sources.push(Source {
                        connection: *connection_id,
                        file: file.clone(),
                        keys: BTreeSet::new(),
                        publications: BTreeSet::new(),
                    });
                }
            }
        }
    }

    /// The projection of `source` that the release keeps: only what rendering reads. A link or
    /// embed only when the release publishes its target, and the frontmatter keys that its
    /// publications allow with `slug`. No `publish` value, tag, heading or block id: a build from
    /// the release does not select again, it takes the notes that each publication selected.
    fn projection(&self, source: &Source) -> IndexedFile {
        let published = |target: &Option<String>| {
            target.as_ref().is_some_and(|target| {
                self.sources.iter().any(|other| {
                    other.connection == source.connection && &other.file.path == target
                })
            })
        };
        let mut file = source.file.clone();
        file.links.retain(|link| published(&link.target));
        file.embeds.retain(|link| published(&link.target));
        file.headings.clear();
        file.blocks.clear();
        file.tags.clear();
        file.frontmatter = file.frontmatter.map(|frontmatter| {
            frontmatter
                .into_iter()
                .filter(|(key, _)| key == "slug" || source.keys.contains(key))
                .collect()
        });
        file
    }

    /// Write the sources of the release `release_id`: the projection of each published note and
    /// attachment.
    ///
    /// # Errors
    ///
    /// The query error of a row that cannot be written.
    pub(crate) async fn write(
        &self,
        connection: &mut PgConnection,
        release_id: &str,
    ) -> Result<(), sqlx::Error> {
        for source in &self.sources {
            sqlx::query(
                "insert into sites.release_sources (release_id, connection_id, path, file, \
                 publication_ids) values ($1, $2, $3, $4, $5) on conflict do nothing",
            )
            .bind(release_id)
            .bind(source.connection)
            .bind(&source.file.path)
            .bind(sqlx::types::Json(self.projection(source)))
            .bind(source.publications.iter().cloned().collect::<Vec<_>>())
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
