//! Which publications put each page and each asset into a release.
//!
//! A note can be selected by more than one publication of a site. The delivery routes serve a page,
//! an asset or a search hit only while one of its publications is in the effective set, so a
//! publication that the owner removes leaves the delivery at once, before the next build
//! (workspace spec `site-manifest`, "a removal stops at once").

use std::collections::{BTreeMap, BTreeSet};

use ratatoskr_sites_publications::{
    BuildError, IndexedFile, assign_slugs, select, tag_key, tag_matches,
};
use ratatoskr_sites_release::{
    Built, PublicationInput, SiteInput, may_publish_attachment, selected_notes,
};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

/// One published file of a release and what its projection may keep.
#[derive(Debug, Clone)]
struct Source {
    connection: Uuid,
    file: IndexedFile,
    /// The frontmatter keys that the page of the note rendered in this release.
    keys: BTreeSet<String>,
    /// The publications that publish the file with the stored revision: for a note those that
    /// select it at that revision, for an attachment those that select a note that uses it, have it
    /// at that revision among their own files, and may publish it.
    publications: BTreeSet<String>,
    /// Whether the `publish` value of the note names the site: `true`, the site id, or a list
    /// with the site id.
    names_site: bool,
    /// The tags of the note, in the form that the selection compares (without `#`), that matched an
    /// included tag of a selecting publication.
    selection_tags: BTreeSet<String>,
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
            for file in input
                .files
                .iter()
                .filter(|file| chosen.contains(&file.path))
            {
                let same = |source: &Source| {
                    source.connection == *connection_id && source.file.path == file.path
                };
                if !sources.iter().any(same) {
                    sources.push(Source {
                        connection: *connection_id,
                        file: file.clone(),
                        keys: BTreeSet::new(),
                        publications: BTreeSet::new(),
                        names_site: false,
                        selection_tags: BTreeSet::new(),
                    });
                }
                let Some(source) = sources.iter_mut().find(|source| same(source)) else {
                    continue;
                };
                // A publication publishes the note only when its own file is the revision that
                // the row stores (that of the first selecting publication, which renders the
                // page): a manual publication that reads an older revision from the release does
                // not publish a newer one that an automatic publication read.
                if file.revision != source.file.revision {
                    continue;
                }
                by_path
                    .entry(file.path.clone())
                    .or_default()
                    .insert(publication_id.clone());
                source.publications.insert(publication_id.clone());
                // The selection inputs come from the revision that the row stores (that of the
                // first selecting publication), never from the version that another publication
                // read.
                source.names_site = names_site(&source.file, site.site_id.as_str());
                let tags = matched_tags(&source.file, &input.publication.included_tags);
                source.selection_tags.extend(tags);
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
            // Only the keys that the page rendered: a build from the release renders those that
            // the current allowlist still allows, and never a key that the page did not show.
            source.keys = page.map_or_else(BTreeSet::new, |page| {
                page.page.frontmatter.keys().cloned().collect()
            });
        }
        let assets: BTreeSet<&str> = built
            .assets
            .iter()
            .map(|asset| asset.digest.hex.as_str())
            .collect();
        // Each target of a resolved link or embed, with the publications that selected a note
        // that names it.
        let mut targets: BTreeMap<(Uuid, String), BTreeSet<String>> = BTreeMap::new();
        for source in &self.sources {
            for link in source.file.links.iter().chain(&source.file.embeds) {
                if let Some(target) = &link.target {
                    targets
                        .entry((source.connection, target.clone()))
                        .or_default()
                        .extend(source.publications.iter().cloned());
                }
            }
        }
        for ((connection_id, _), input) in publications.iter().zip(&site.publications) {
            for file in input.files {
                if file.is_markdown() || !assets.contains(file.revision.digest.hex.as_str()) {
                    continue;
                }
                let Some(selecting) = targets.get(&(*connection_id, file.path.clone())) else {
                    continue;
                };
                // Only a publication that selected a note that uses the attachment, that has the
                // attachment among its own files (for a publication read from the release, the
                // attachments that it published), and that may publish it, publishes it.
                let allowing: BTreeSet<String> = publications
                    .iter()
                    .zip(&site.publications)
                    .filter(|((connection, id), other)| {
                        connection == connection_id
                            && selecting.contains(id)
                            && publishes_attachment(other, file)
                    })
                    .map(|((_, id), _)| id.clone())
                    .collect();
                let known = self.sources.iter().any(|source| {
                    source.connection == *connection_id && source.file.path == file.path
                });
                if !allowing.is_empty() && !known {
                    self.sources.push(Source {
                        connection: *connection_id,
                        file: file.clone(),
                        keys: BTreeSet::new(),
                        publications: allowing,
                        names_site: false,
                        selection_tags: BTreeSet::new(),
                    });
                }
            }
        }
    }

    /// The projection of `source` that the release keeps: only what rendering reads. The links and
    /// embeds that its page resolved, and the frontmatter keys that its page rendered with `slug`,
    /// never `publish`. No tag, heading or block id: the selection inputs are the columns
    /// `names_site` and `selection_tags`.
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
                .filter(|(key, _)| key == "slug" || (key != "publish" && source.keys.contains(key)))
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
                 publication_ids, names_site, selection_tags) \
                 values ($1, $2, $3, $4, $5, $6, $7) on conflict do nothing",
            )
            .bind(release_id)
            .bind(source.connection)
            .bind(&source.file.path)
            .bind(sqlx::types::Json(self.projection(source)))
            .bind(source.publications.iter().cloned().collect::<Vec<_>>())
            .bind(source.names_site)
            .bind(source.selection_tags.iter().cloned().collect::<Vec<_>>())
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
    }

    /// Write again the provenance of the release `release_id`, which a build made again: its
    /// sources, and the publications of its pages, search rows and assets.
    ///
    /// # Errors
    ///
    /// The query error of a row that cannot be written.
    pub(crate) async fn rewrite(
        &self,
        connection: &mut PgConnection,
        release_id: &str,
        built: &Built,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("delete from sites.release_sources where release_id = $1")
            .bind(release_id)
            .execute(&mut *connection)
            .await?;
        self.write(connection, release_id).await?;
        for page in &built.pages {
            let slug = page.page.slug.as_str();
            for table in ["release_pages", "release_search"] {
                sqlx::query(&format!(
                    "update sites.{table} set publication_ids = $3 \
                     where release_id = $1 and slug = $2"
                ))
                .bind(release_id)
                .bind(slug)
                .bind(self.page(slug))
                .execute(&mut *connection)
                .await?;
            }
        }
        // An asset that no attachment of this build names keeps no publication of an earlier one.
        sqlx::query("update sites.release_assets set publication_ids = '{}' where release_id = $1")
            .bind(release_id)
            .execute(&mut *connection)
            .await?;
        for (digest, publications) in self.assets(built) {
            sqlx::query(
                "update sites.release_assets set publication_ids = $3 \
                 where release_id = $1 and digest = $2",
            )
            .bind(release_id)
            .bind(digest)
            .bind(publications)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
    }

    /// The publications of the page `slug`.
    pub(crate) fn page(&self, slug: &str) -> &[String] {
        self.pages.get(slug).map_or(&[], Vec::as_slice)
    }

    /// The publications of each asset of `built`: those of its attachments, each a publication that
    /// selected a note that uses it and that does not exclude it.
    pub(crate) fn assets(&self, built: &Built) -> BTreeMap<String, Vec<String>> {
        let used: BTreeSet<&str> = built
            .pages
            .iter()
            .flat_map(|page| page.page.assets.iter().map(|asset| asset.sha256.as_str()))
            .collect();
        let mut assets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for source in self
            .sources
            .iter()
            .filter(|source| !source.file.is_markdown())
        {
            let digest = source.file.revision.digest.hex.as_str();
            if used.contains(digest) {
                assets
                    .entry(digest.to_owned())
                    .or_default()
                    .extend(source.publications.iter().cloned());
            }
        }
        assets
            .into_iter()
            .map(|(digest, publications)| (digest, publications.into_iter().collect()))
            .collect()
    }
}

/// Whether the `publish` value of `file` names the site `site_id`, as the selection reads it.
fn names_site(file: &IndexedFile, site_id: &str) -> bool {
    match file.frontmatter_value("publish") {
        Some(Value::Bool(true)) => true,
        Some(Value::String(named)) => named == site_id,
        Some(Value::Array(sites)) => sites.iter().any(|named| named.as_str() == Some(site_id)),
        _ => false,
    }
}

/// The tags of `file`, in the form that the selection compares ([`tag_key`]), that match one of
/// `included`.
fn matched_tags(file: &IndexedFile, included: &[String]) -> Vec<String> {
    file.tags
        .iter()
        .filter(|tag| included.iter().any(|included| tag_matches(included, tag)))
        .map(|tag| tag_key(tag).to_owned())
        .collect()
}

/// Whether `input` has `file`, the same path at the same revision, among its own files and may
/// publish it.
fn publishes_attachment(input: &PublicationInput<'_>, file: &IndexedFile) -> bool {
    input.files.iter().any(|own| {
        own.path == file.path
            && own.revision == file.revision
            && may_publish_attachment(&input.publication, own)
    })
}
