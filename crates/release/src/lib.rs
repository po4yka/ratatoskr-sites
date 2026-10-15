//! The release builder: from the selected notes of the publications of one site to an immutable
//! release (workspace design D4 and D5, specs `site-publications` and `site-release-privacy`).
//!
//! This crate reads no file and no database. The caller gives it the index of each connection and
//! the text of each selected note, and stores what it returns. Every build parses, filters and
//! renders every selected page; nothing of an earlier build is reused.
//!
//! The order is fixed: the text of a note loses its frontmatter and its comments (`source`), is
//! parsed into one tree (`tree`), passes the privacy filter (`filter`), and only then reaches the
//! page tree and HTML renderers (`render`) and the link and asset collectors below.

mod filter;
mod render;
mod source;
mod tree;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use ratatoskr_identifiers::{BlobRef, DigestHex};
use ratatoskr_sites_contracts::{
    AssetRef, FrontmatterKey, NavigationEntry, Page, PageRoot, PageRootType, PageTitle,
    PublicationId, Release, ReleaseAsset, ReleasePage, SiteId, Slug,
};
use ratatoskr_sites_publications::{BuildError, IndexedFile, Publication, assign_slugs, select};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::filter::Scope;
use crate::tree::Node;

/// The frontmatter fields that every publication publishes (workspace spec
/// `site-release-privacy`). The owner may add names to a publication.
pub const DEFAULT_FRONTMATTER_ALLOWLIST: [&str; 7] = [
    "title",
    "description",
    "tags",
    "aliases",
    "date",
    "updated",
    "cover",
];

/// The frontmatter fields that the builder reads and never publishes.
const BUILDER_FIELDS: [&str; 2] = ["publish", "slug"];

/// The class of a selected note whose text the caller did not give.
pub const NOTE_UNAVAILABLE: &str = "sites.note_unavailable";

/// The class of a page that does not satisfy its contract.
pub const PAGE_INVALID: &str = "sites.page_invalid";

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
    /// The stored bytes of the page document: the `serde_json` form of `page`.
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

/// One selected note and the publication that selects it.
#[derive(Debug, Clone, Copy)]
struct Selected<'a> {
    input: &'a PublicationInput<'a>,
    file: &'a IndexedFile,
}

/// The selected notes of every publication, each once per vault connection and path.
fn selection<'a>(site: &'a SiteInput<'a>) -> Vec<Selected<'a>> {
    let mut seen = BTreeSet::new();
    let mut notes = Vec::new();
    for input in &site.publications {
        let chosen = select(&input.publication, input.files);
        for file in input
            .files
            .iter()
            .filter(|file| chosen.contains(&file.path))
        {
            if seen.insert((input.connection_id.as_str(), file.path.as_str())) {
                notes.push(Selected { input, file });
            }
        }
    }
    notes
}

/// The Markdown notes that the build reads: the selected notes of every publication.
#[must_use]
pub fn selected_notes<'a>(site: &'a SiteInput<'a>) -> Vec<&'a IndexedFile> {
    selection(site).into_iter().map(|note| note.file).collect()
}

/// Build the release of `site`. `texts` holds the text of each selected note by the hex SHA-256 of
/// its revision; the text of an unselected note is never read.
///
/// # Errors
///
/// [`BuildError`] when the build makes no release: a slug collision or an invalid slug, a selected
/// note whose text is missing, or a page that does not satisfy its contract.
pub fn build_release(
    site: &SiteInput<'_>,
    texts: &BTreeMap<String, String>,
) -> Result<Built, BuildError> {
    let notes = selection(site);
    let files: Vec<&IndexedFile> = notes.iter().map(|note| note.file).collect();
    let slugs = assign_slugs(&files)?;
    let by_path: BTreeMap<&str, &Slug> = slugs
        .iter()
        .map(|(slug, path)| (path.as_str(), slug))
        .collect();

    let mut rendered = Vec::with_capacity(notes.len());
    for note in &notes {
        let slug = by_path
            .get(note.file.path.as_str())
            .copied()
            .ok_or_else(|| invalid(&note.file.path))?;
        let text = texts
            .get(note.file.revision.digest.hex.as_str())
            .ok_or_else(|| BuildError {
                class: NOTE_UNAVAILABLE,
                detail: format!("the text of {} is not available", note.file.path),
            })?;
        rendered.push(render_note(note, &notes, &by_path, slug, text)?);
    }
    assemble(site, rendered)
}

/// One note after the filter and the renderers, before the backlinks are known.
struct Rendered {
    slug: Slug,
    title: PageTitle,
    frontmatter: BTreeMap<String, Value>,
    ast: PageRoot,
    html: String,
    links: BTreeSet<Slug>,
    assets: BTreeMap<String, AssetRef>,
    blobs: Vec<BlobRef>,
}

fn render_note(
    note: &Selected<'_>,
    notes: &[Selected<'_>],
    by_path: &BTreeMap<&str, &Slug>,
    slug: &Slug,
    text: &str,
) -> Result<Rendered, BuildError> {
    let connection = note.input.connection_id.as_str();
    let pages: BTreeMap<String, Slug> = notes
        .iter()
        .filter(|other| other.input.connection_id == connection)
        .filter_map(|other| {
            by_path
                .get(other.file.path.as_str())
                .map(|slug| (other.file.path.clone(), (*slug).clone()))
        })
        .collect();
    let attachments = publishable_attachments(&note.input.publication, note.input.files);
    let scope = Scope {
        file: note.file,
        pages: &pages,
        attachments: &attachments,
    };
    let body = source::without_comments(source::body_of(text));
    let tree = filter::filter(tree::parse(&body), &scope);

    let mut links = BTreeSet::new();
    let mut assets = BTreeMap::new();
    collect(&tree, &mut links, &mut assets);
    let blobs = attachments
        .values()
        .filter(|blob| assets.contains_key(blob.digest.hex.as_str()))
        .cloned()
        .collect();
    Ok(Rendered {
        slug: slug.clone(),
        title: title_of(note.file)?,
        frontmatter: allowed_frontmatter(note.file, &note.input.frontmatter_allow),
        ast: PageRoot {
            node_type: PageRootType::Root,
            children: render::to_ast(&tree),
        },
        html: render::to_html(&tree),
        links,
        assets,
        blobs,
    })
}

/// The attachments that `publication` may publish when a selected page uses them: every file of
/// the connection that is not Markdown, not under `.obsidian/` and not in an excluded folder.
fn publishable_attachments(
    publication: &Publication,
    files: &[IndexedFile],
) -> BTreeMap<String, BlobRef> {
    files
        .iter()
        .filter(|file| !file.is_markdown())
        .filter(|file| {
            !in_folder(&file.path, ".obsidian")
                && !publication
                    .excluded_folders
                    .iter()
                    .any(|folder| in_folder(&file.path, folder))
        })
        .map(|file| (file.path.clone(), file.revision.clone()))
        .collect()
}

fn in_folder(path: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    !folder.is_empty()
        && path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The slugs that the filtered tree links to or embeds, and the assets that it embeds.
fn collect(nodes: &[Node], links: &mut BTreeSet<Slug>, assets: &mut BTreeMap<String, AssetRef>) {
    for node in nodes {
        match node {
            Node::PageLink { slug, children, .. } => {
                links.insert(slug.clone());
                collect(children, links, assets);
            }
            Node::PageEmbed { slug, .. } => {
                links.insert(slug.clone());
            }
            Node::AssetEmbed {
                sha256, media_type, ..
            } => {
                assets.insert(
                    sha256.as_str().to_owned(),
                    AssetRef {
                        sha256: sha256.clone(),
                        media_type: media_type.clone(),
                    },
                );
            }
            Node::Callout {
                title, children, ..
            } => {
                collect(title, links, assets);
                collect(children, links, assets);
            }
            other => collect(children_of(other), links, assets),
        }
    }
}

fn children_of(node: &Node) -> &[Node] {
    match node {
        Node::Paragraph(children)
        | Node::Blockquote(children)
        | Node::TableRow(children)
        | Node::TableCell(children)
        | Node::Emphasis(children)
        | Node::Strong(children)
        | Node::Delete(children)
        | Node::Heading { children, .. }
        | Node::List { children, .. }
        | Node::ListItem { children, .. }
        | Node::Table { children, .. }
        | Node::ExternalLink { children, .. } => children,
        _ => &[],
    }
}

/// The frontmatter `title` when it is a valid title, or else the file name without `.md`.
fn title_of(file: &IndexedFile) -> Result<PageTitle, BuildError> {
    if let Some(Value::String(title)) = file.frontmatter_value("title")
        && let Ok(title) = PageTitle::parse(title)
    {
        return Ok(title);
    }
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    PageTitle::parse(stem)
        .or_else(|_| PageTitle::parse("Untitled"))
        .map_err(|_| invalid(&file.path))
}

/// The frontmatter fields of `file` on the allowlist, without the fields of the builder.
fn allowed_frontmatter(file: &IndexedFile, extra: &[String]) -> BTreeMap<String, Value> {
    let Some(frontmatter) = &file.frontmatter else {
        return BTreeMap::new();
    };
    frontmatter
        .iter()
        .filter(|(key, _)| {
            let key = key.as_str();
            !BUILDER_FIELDS.contains(&key)
                && (DEFAULT_FRONTMATTER_ALLOWLIST.contains(&key)
                    || extra.iter().any(|allowed| allowed == key))
                && FrontmatterKey::parse(key).is_ok()
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn invalid(path: &str) -> BuildError {
    BuildError {
        class: PAGE_INVALID,
        detail: format!("{path} makes no valid page"),
    }
}

/// The pages with their backlinks, and the release document.
fn assemble(site: &SiteInput<'_>, rendered: Vec<Rendered>) -> Result<Built, BuildError> {
    let mut backlinks: BTreeMap<Slug, BTreeSet<Slug>> = BTreeMap::new();
    for page in &rendered {
        for target in &page.links {
            backlinks
                .entry(target.clone())
                .or_default()
                .insert(page.slug.clone());
        }
    }
    let mut blobs: BTreeMap<String, BlobRef> = BTreeMap::new();
    let mut pages = Vec::with_capacity(rendered.len());
    for page in rendered {
        for blob in &page.blobs {
            blobs.insert(blob.digest.hex.as_str().to_owned(), blob.clone());
        }
        let page = Page {
            backlinks: backlinks
                .remove(&page.slug)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            slug: page.slug,
            title: page.title,
            frontmatter: page.frontmatter,
            ast: page.ast,
            html: page.html,
            links: page.links.into_iter().collect(),
            assets: page.assets.into_values().collect(),
        };
        let bytes = serde_json::to_vec(&page).map_err(|_| invalid(page.slug.as_str()))?;
        pages.push(BuiltPage { page, bytes });
    }
    pages.sort_by(|left, right| left.page.slug.cmp(&right.page.slug));
    let release = release_document(site, &pages, &blobs)?;
    Ok(Built {
        release,
        pages,
        assets: blobs.into_values().collect(),
    })
}

fn release_document(
    site: &SiteInput<'_>,
    pages: &[BuiltPage],
    blobs: &BTreeMap<String, BlobRef>,
) -> Result<Release, BuildError> {
    let publication_ids: BTreeSet<PublicationId> = site
        .publications
        .iter()
        .map(|input| input.publication.publication_id.clone())
        .collect();
    let mut release_pages = Vec::with_capacity(pages.len());
    for built in pages {
        release_pages.push(ReleasePage {
            slug: built.page.slug.clone(),
            title: built.page.title.clone(),
            page_sha256: DigestHex::parse(&sha256_hex(&built.bytes))
                .map_err(|_| invalid(built.page.slug.as_str()))?,
        });
    }
    let release = Release {
        site_id: site.site_id.clone(),
        publication_ids: publication_ids.into_iter().collect(),
        navigation: pages
            .iter()
            .map(|built| NavigationEntry {
                title: built.page.title.clone(),
                slug: Some(built.page.slug.clone()),
                children: Vec::new(),
            })
            .collect(),
        pages: release_pages,
        assets: blobs
            .values()
            .map(|blob| ReleaseAsset {
                sha256: blob.digest.hex.clone(),
                media_type: blob.media_type.clone(),
                size_bytes: blob.length_bytes,
            })
            .collect(),
    };
    release.validate().map_err(|error| BuildError {
        class: PAGE_INVALID,
        detail: error.to_string(),
    })?;
    Ok(release)
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
