//! The release builder: from the selected notes of the publications of one site to an immutable
//! release (workspace design D4 and D5, specs `site-publications` and `site-release-privacy`).
//!
//! This crate reads no file and no database. The caller gives it the index of each connection and
//! the text of each selected note, and stores what it returns.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ratatoskr_identifiers::BlobRef;
use ratatoskr_sites_contracts::{
    NavigationEntry, Page, PageNode, PageRoot, PageRootType, PageTitle, Release, ReleasePage,
    SiteId,
};
use ratatoskr_sites_publications::{BuildError, IndexedFile, Publication, assign_slugs};
use sha2::{Digest as _, Sha256};

/// One publication of the site, with the index of its vault connection.
#[derive(Debug, Clone)]
pub struct PublicationInput<'a> {
    /// The publication.
    pub publication: Publication,
    /// The vault connection that the publication reads.
    pub connection_id: String,
    /// The frontmatter fields that the owner adds to the default allowlist.
    pub frontmatter_allow: Vec<String>,
    /// The index of the connection.
    pub files: &'a [IndexedFile],
}

/// The publications of one site.
#[derive(Debug, Clone)]
pub struct SiteInput<'a> {
    /// The site.
    pub site_id: SiteId,
    /// Its publications.
    pub publications: Vec<PublicationInput<'a>>,
}

/// One rendered page and the bytes that the release names by their SHA-256.
#[derive(Debug, Clone)]
pub struct BuiltPage {
    /// The page document.
    pub page: Page,
    /// The stored bytes of the page document.
    pub bytes: Vec<u8>,
}

/// What a build makes: the release document, its pages and the assets that its pages use.
#[derive(Debug, Clone)]
pub struct Built {
    /// The release document.
    pub release: Release,
    /// The pages, in the order of their slugs.
    pub pages: Vec<BuiltPage>,
    /// The notes blobs of the assets, in the order of their SHA-256.
    pub assets: Vec<BlobRef>,
}

/// The Markdown notes that the build reads.
#[must_use]
pub fn selected_notes<'a>(site: &SiteInput<'a>) -> Vec<&'a IndexedFile> {
    site.publications
        .iter()
        .flat_map(|input| input.files.iter().filter(|file| file.is_markdown()))
        .collect()
}

/// Build the release of `site`. `texts` holds the text of each note by the hex SHA-256 of its
/// revision.
///
/// # Errors
///
/// [`BuildError`] when the build makes no release.
pub fn build_release(
    site: &SiteInput<'_>,
    texts: &BTreeMap<String, String>,
) -> Result<Built, BuildError> {
    let notes = selected_notes(site);
    let slugs = assign_slugs(&notes)?;
    let mut pages = Vec::new();
    for (slug, path) in slugs {
        let Some(file) = notes.iter().find(|file| file.path == path) else {
            continue;
        };
        let source = texts
            .get(file.revision.digest.hex.as_str())
            .cloned()
            .unwrap_or_default();
        let page = Page {
            slug,
            title: PageTitle::parse(&path).map_err(|_| BuildError {
                class: "sites.page_invalid",
                detail: path.clone(),
            })?,
            frontmatter: file
                .frontmatter
                .clone()
                .map(|map| map.into_iter().collect())
                .unwrap_or_default(),
            ast: PageRoot {
                node_type: PageRootType::Root,
                children: vec![PageNode::Text {
                    value: source.clone(),
                }],
            },
            html: source,
            links: Vec::new(),
            backlinks: Vec::new(),
            assets: Vec::new(),
        };
        let bytes = serde_json::to_vec(&page).map_err(|error| BuildError {
            class: "sites.page_invalid",
            detail: error.to_string(),
        })?;
        pages.push(BuiltPage { page, bytes });
    }
    let release = Release {
        site_id: site.site_id.clone(),
        publication_ids: site
            .publications
            .iter()
            .map(|input| input.publication.publication_id.clone())
            .collect(),
        pages: pages
            .iter()
            .map(|built| -> Result<ReleasePage, BuildError> {
                Ok(ReleasePage {
                    slug: built.page.slug.clone(),
                    title: built.page.title.clone(),
                    page_sha256: ratatoskr_identifiers::DigestHex::parse(&sha256_hex(&built.bytes))
                        .map_err(|_| BuildError {
                            class: "sites.page_invalid",
                            detail: built.page.slug.as_str().to_owned(),
                        })?,
                })
            })
            .collect::<Result<_, _>>()?,
        assets: Vec::new(),
        navigation: Vec::<NavigationEntry>::new(),
    };
    Ok(Built {
        release,
        pages,
        assets: Vec::new(),
    })
}

/// The lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
