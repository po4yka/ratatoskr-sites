//! SITE-025 task 5.14: a release holds nothing from a file that its publication does not select
//! (workspace spec `site-release-privacy`).
//!
//! The fixture vault `fixtures/privacy-vault/` is one vault connection. The publication reads the
//! whole vault in the mode `all` and excludes `private/`; `Diary zq7diaryname.md` has
//! `publish: false`. Every string with `zq7` comes from an unselected file.

use std::collections::BTreeMap;
use std::path::Path;

use ratatoskr_sites_contracts::{Page, PublicationId, SiteId};
use ratatoskr_sites_publications::test_support::load_index;
use ratatoskr_sites_publications::{IndexedFile, Publication, PublicationMode};
use ratatoskr_sites_release::{Built, PublicationInput, SiteInput, build_release};
use serde_json::Value;

type Outcome = Result<(), Box<dyn std::error::Error>>;
type Failure = Box<dyn std::error::Error>;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/privacy-vault");

/// The prefix of every canary of an unselected file.
const CANARY: &str = "zq7";

fn index() -> Result<Vec<IndexedFile>, Failure> {
    Ok(load_index(Path::new(FIXTURE))?)
}

/// The text of every note of the fixture, by the hex SHA-256 of its bytes. It reads the files of
/// the fixture as they are on disk, so a test that moves a file in the index still finds its text.
fn texts() -> Result<BTreeMap<String, String>, Failure> {
    let mut texts = BTreeMap::new();
    for file in index()?.iter().filter(|file| file.is_markdown()) {
        let text = std::fs::read_to_string(Path::new(FIXTURE).join("vault").join(&file.path))?;
        texts.insert(file.revision.digest.hex.as_str().to_owned(), text);
    }
    Ok(texts)
}

fn build(files: &[IndexedFile]) -> Result<Built, Failure> {
    let site = SiteInput {
        site_id: SiteId::parse("site-p")?,
        publications: vec![PublicationInput {
            publication: Publication {
                site_id: SiteId::parse("site-p")?,
                publication_id: PublicationId::parse("pub-p")?,
                mode: PublicationMode::All,
                included_folders: Vec::new(),
                excluded_folders: vec!["private".to_owned()],
                included_tags: Vec::new(),
            },
            connection_id: "connection-p".to_owned(),
            frontmatter_allow: Vec::new(),
            files,
        }],
    };
    Ok(build_release(&site, &texts()?)?)
}

fn page<'a>(built: &'a Built, slug: &str) -> Result<&'a Page, Failure> {
    built
        .pages
        .iter()
        .map(|built| &built.page)
        .find(|page| page.slug.as_str() == slug)
        .ok_or_else(|| format!("the release has no page {slug}").into())
}

/// Every byte of the release: the document, the pages and the bytes of each asset.
fn release_bytes(built: &Built) -> Result<Vec<u8>, Failure> {
    let mut bytes = serde_json::to_vec(&built.release)?;
    for page in &built.pages {
        bytes.extend_from_slice(&page.bytes);
    }
    for asset in &built.assets {
        let file = index()?
            .into_iter()
            .find(|file| file.revision.digest == asset.digest)
            .ok_or("an asset of the release is not in the fixture")?;
        bytes.extend(std::fs::read(
            Path::new(FIXTURE).join("vault").join(&file.path),
        )?);
    }
    Ok(bytes)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// The `type` of every node of a page tree.
fn node_types(page: &Page) -> Result<Vec<String>, Failure> {
    fn walk(value: &Value, types: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                if let Some(Value::String(kind)) = object.get("type") {
                    types.push(kind.clone());
                }
                object.values().for_each(|value| walk(value, types));
            }
            Value::Array(items) => items.iter().for_each(|value| walk(value, types)),
            _ => {}
        }
    }
    let mut types = Vec::new();
    walk(&serde_json::to_value(&page.ast)?, &mut types);
    Ok(types)
}

fn slugs(list: &[ratatoskr_sites_contracts::Slug]) -> Vec<&str> {
    list.iter()
        .map(ratatoskr_sites_contracts::Slug::as_str)
        .collect()
}

#[test]
fn no_canary_from_an_unselected_file_reaches_the_release() -> Outcome {
    let built = build(&index()?)?;
    let bytes = release_bytes(&built)?;
    assert!(
        !contains(&bytes, CANARY),
        "a string of an unselected file reached the release: {}",
        String::from_utf8_lossy(&bytes)
    );
    Ok(())
}

#[test]
fn link_inside_comment_makes_no_edge() -> Outcome {
    let built = build(&index()?)?;
    assert_eq!(
        slugs(&page(&built, "other")?.backlinks),
        ["home"],
        "only home links to other; the link inside the comment of comments makes no backlink"
    );
    assert!(
        page(&built, "comments")?.links.is_empty(),
        "the link inside the comment makes no outgoing link"
    );
    Ok(())
}

#[test]
fn aliased_link_to_unselected_note_is_plain_alias() -> Outcome {
    let built = build(&index()?)?;
    let page = page(&built, "alias-private")?;
    let types = node_types(page)?;
    assert!(
        !types
            .iter()
            .any(|kind| kind == "wikiLink" || kind == "link"),
        "the link to the unselected note is no link node: {types:?}"
    );
    let ast = serde_json::to_string(&page.ast)?;
    assert!(ast.contains("my notes"), "the alias stays: {ast}");
    assert!(
        !page.html.contains("<a"),
        "the html holds no link: {}",
        page.html
    );
    assert!(page.html.contains("my notes"), "the html holds the alias");
    for leak in ["Salary", CANARY] {
        assert!(
            !ast.contains(leak) && !page.html.contains(leak),
            "nothing of the target reaches the page: {leak}"
        );
    }
    Ok(())
}

#[test]
fn unselected_target_renders_like_missing_target() -> Outcome {
    let built = build(&index()?)?;
    let private = page(&built, "alias-private")?;
    let missing = page(&built, "alias-missing")?;
    assert_eq!(private.ast, missing.ast, "the two trees are identical");
    assert_eq!(private.html, missing.html, "the two bodies are identical");
    Ok(())
}

#[test]
fn embed_of_unselected_note_leaves_no_trace() -> Outcome {
    let built = build(&index()?)?;
    let embeds = page(&built, "embeds")?;
    let without = page(&built, "embeds-without")?;
    assert_eq!(
        embeds.ast, without.ast,
        "the tree is the page without the embed"
    );
    assert_eq!(
        embeds.html, without.html,
        "the html is the page without the embed"
    );
    Ok(())
}

#[test]
fn obsidian_comment_is_removed() -> Outcome {
    let built = build(&index()?)?;
    let page = page(&built, "comments")?;
    let ast = serde_json::to_string(&page.ast)?;
    for secret in [
        "COMMENT-SECRET",
        "HTML-COMMENT-SECRET",
        "three lines",
        "see [[",
    ] {
        assert!(
            !ast.contains(secret) && !page.html.contains(secret),
            "the comment text {secret} is removed"
        );
    }
    assert!(
        ast.contains("Visible text after the comments."),
        "the text around the comments stays: {ast}"
    );
    Ok(())
}

#[test]
fn comment_marker_in_code_is_kept() -> Outcome {
    let built = build(&index()?)?;
    let ast = serde_json::to_value(&page(&built, "code")?.ast)?;
    let code = ast
        .pointer("/children/1")
        .ok_or("the code page has a second block")?;
    assert_eq!(code.get("type"), Some(&Value::from("code")), "{ast}");
    assert_eq!(code.get("lang"), Some(&Value::from("text")), "{ast}");
    assert_eq!(
        code.get("value"),
        Some(&Value::from("%% not a comment %%\n")),
        "the fenced block keeps the comment marker unchanged: {ast}"
    );
    assert!(
        ast.to_string().contains("\"%% inline code %%\""),
        "the inline code keeps the comment marker: {ast}"
    );
    Ok(())
}

#[test]
fn frontmatter_outside_allowlist_is_dropped() -> Outcome {
    let built = build(&index()?)?;
    let home = built
        .pages
        .iter()
        .find(|built| built.page.slug.as_str() == "home")
        .ok_or("the release has a page home")?;
    assert_eq!(
        home.page.frontmatter.keys().collect::<Vec<_>>(),
        ["title"],
        "only the allowlisted title is published"
    );
    assert!(
        !contains(&home.bytes, "owner-marker") && !contains(&home.bytes, "\"publish\""),
        "the dropped fields are nowhere in the page"
    );
    Ok(())
}

#[test]
fn attachment_of_unselected_note_is_not_published() -> Outcome {
    let files = index()?;
    let plan = files
        .iter()
        .find(|file| file.path == "img/plan.png")
        .ok_or("the fixture holds img/plan.png")?;
    let digest = plan.revision.digest.hex.as_str();
    let built = build(&files)?;
    assert!(
        built
            .release
            .assets
            .iter()
            .all(|asset| asset.sha256.as_str() != digest),
        "the release has no asset with the digest of img/plan.png"
    );
    assert!(
        !contains(&release_bytes(&built)?, digest),
        "no byte of the release names the digest of img/plan.png"
    );
    Ok(())
}

#[test]
fn asset_url_carries_no_vault_path() -> Outcome {
    let files = index()?;
    let scan = files
        .iter()
        .find(|file| file.path == "attachments/2026/passport-scan-blurred.png")
        .ok_or("the fixture holds the scan")?;
    let digest = scan.revision.digest.hex.as_str();
    let built = build(&files)?;
    let page = page(&built, "assets")?;
    assert!(
        page.html.contains(&format!("assets/{digest}")),
        "the page refers to the asset by its digest: {}",
        page.html
    );
    assert_eq!(
        page.assets
            .iter()
            .map(|asset| asset.sha256.as_str())
            .collect::<Vec<_>>(),
        [digest],
        "the page uses the asset"
    );
    assert!(
        built
            .release
            .assets
            .iter()
            .any(|asset| asset.sha256.as_str() == digest),
        "the release publishes the asset"
    );
    assert!(
        !contains(&release_bytes(&built)?, "passport-scan"),
        "the vault path of the asset is nowhere in the release"
    );
    Ok(())
}

#[test]
fn target_unselected_after_release_rerenders_linking_page() -> Outcome {
    let mut files = index()?;
    let first = build(&files)?;
    assert_eq!(slugs(&page(&first, "a")?.links), ["b"], "a links to b");

    // `b` gets `publish: false`; `a` does not change.
    let b = files
        .iter_mut()
        .find(|file| file.path == "b.md")
        .ok_or("the fixture holds b.md")?;
    b.frontmatter
        .get_or_insert_with(Default::default)
        .insert("publish".to_owned(), Value::Bool(false));
    let second = build(&files)?;

    let a = page(&second, "a")?;
    assert!(a.links.is_empty(), "a no longer links to b");
    let types = node_types(a)?;
    assert!(
        !types.iter().any(|kind| kind == "wikiLink"),
        "the link in a is plain text: {types:?}"
    );
    assert!(
        !contains(&release_bytes(&second)?, "yk4b"),
        "the next release holds nothing from b"
    );
    Ok(())
}

#[test]
fn backlink_from_unselected_page_is_dropped_on_rebuild() -> Outcome {
    let mut files = index()?;
    let first = build(&files)?;
    assert_eq!(
        slugs(&page(&first, "d")?.backlinks),
        ["c"],
        "c is a backlink of d"
    );

    // `c` moves into the excluded folder; `d` does not change.
    let c = files
        .iter_mut()
        .find(|file| file.path == "c.md")
        .ok_or("the fixture holds c.md")?;
    "private/c.md".clone_into(&mut c.path);
    let second = build(&files)?;

    assert!(
        page(&second, "d")?.backlinks.is_empty(),
        "d lists no backlink from c"
    );
    assert!(
        !contains(&release_bytes(&second)?, "yk4c"),
        "the next release holds nothing from c"
    );
    Ok(())
}

#[test]
fn asset_metadata_comes_from_the_used_attachment() -> Outcome {
    let mut files = index()?;
    let scan = files
        .iter()
        .find(|file| file.path == "attachments/2026/passport-scan-blurred.png")
        .ok_or("the fixture holds the scan")?
        .clone();
    // Another attachment with the same bytes and another media type, which no page uses.
    let mut twin = scan.clone();
    "zz/twin.bin".clone_into(&mut twin.path);
    twin.revision.media_type = ratatoskr_identifiers::MediaType::parse("application/octet-stream")?;
    files.push(twin);

    let built = build(&files)?;
    let asset = built
        .release
        .assets
        .iter()
        .find(|asset| asset.sha256 == scan.revision.digest.hex)
        .ok_or("the release publishes the scan")?;
    assert_eq!(
        asset.media_type.as_str(),
        "image/png",
        "the media type is the one of the attachment that the page uses"
    );
    assert_eq!(asset.size_bytes, scan.revision.length_bytes);
    assert_eq!(
        built
            .assets
            .iter()
            .map(|blob| blob.media_type.as_str())
            .collect::<Vec<_>>(),
        ["image/png"],
        "the blob that the build copies is the used attachment"
    );
    Ok(())
}
