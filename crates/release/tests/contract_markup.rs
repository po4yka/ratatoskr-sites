//! SITE-025 task 5.17, security review "Functional" (workspace 5.21h): the renderer and the HTML
//! allowlist write and keep the markup of the contract fixtures of `sites/page.v1`.
//!
//! The four strings below are the `html` of `fixtures/sites/page/valid/*.json` in
//! `ratatoskr-contracts` at 66bdb0c0545363f06137f13687084c21db9b27c5. The contract fixtures win:
//! when the renderer and a fixture disagree, the renderer changes.

use std::collections::BTreeMap;

use ratatoskr_sites_contracts::{PublicationId, SiteId};
use ratatoskr_sites_publications::test_support::revision_of;
use ratatoskr_sites_publications::{
    IndexedFile, IndexedLink, LinkForm, Publication, PublicationMode,
};
use ratatoskr_sites_release::sanitise::sanitise;
use ratatoskr_sites_release::{Built, PublicationInput, SiteInput, build_release};

type Outcome = Result<(), Box<dyn std::error::Error>>;
type Failure = Box<dyn std::error::Error>;

/// `fixtures/sites/page/valid/with-wikilink.json`.
const WITH_WIKILINK: &str = "<p>See <a href=\"handbook/start#start-here\" data-wikilink>the start</a>, my notes and <a href=\"https://example.test/about\">the site</a>.</p>\n";

/// `fixtures/sites/page/valid/with-callout.json`.
const WITH_CALLOUT: &str = "<h1 id=\"start-here\">Start here</h1>\n<div class=\"callout\" data-callout=\"note\" data-callout-fold=\"open\"><div class=\"callout-title\">Before you read</div><div class=\"callout-content\"><p>This handbook is <strong>public</strong>.</p></div></div>\n<ul><li><input type=\"checkbox\" checked disabled> <p>Read the links page.</p></li></ul>\n";

/// `fixtures/sites/page/valid/with-embed.json`.
const WITH_EMBED: &str = "<p><img src=\"assets/ced0e0da2723c17bce89eac3ffc3107ca7753d95c6abc28ce29214a19667728d\" alt=\"The release flow\"></p>\n<p><a href=\"handbook/start#start-here\" data-embed>Start here</a></p>\n";

/// `fixtures/sites/page/valid/with-urls-and-breaks.json`.
const WITH_URLS_AND_BREAKS: &str = "<p>See <a href=\"https://example.test/search?q=site%20release&amp;page=2#results\" title=\"Search\">the search</a><br>\nand the logo.</p>\n<hr>\n<p><img src=\"https://example.test/img/logo%20v2.png?size=small#top\" alt=\"Logo\"></p>\n";

/// The digest in the asset URL of `with-embed.json`.
const FIXTURE_ASSET: &str = "ced0e0da2723c17bce89eac3ffc3107ca7753d95c6abc28ce29214a19667728d";

fn link(link: &str, display: &str, form: LinkForm, target: Option<&str>) -> IndexedLink {
    IndexedLink {
        link: link.to_owned(),
        display_text: display.to_owned(),
        form,
        target: target.map(str::to_owned),
    }
}

fn file(
    path: &str,
    bytes: &[u8],
    media_type: &str,
    links: Vec<IndexedLink>,
    embeds: Vec<IndexedLink>,
) -> Result<IndexedFile, Failure> {
    Ok(IndexedFile {
        path: path.to_owned(),
        revision: revision_of(bytes, media_type)?,
        frontmatter: None,
        tags: Vec::new(),
        headings: Vec::new(),
        blocks: Vec::new(),
        links,
        embeds,
    })
}

const START: &str = "---\ntitle: Start here\n---\n# Start here\n\n> [!note]+ Before you read\n> This handbook is **public**.\n\n- [x] Read the links page.\n";
const LINKS: &str = "See [[handbook/start#Start here|the start]], [[Private|my notes]] and [the site](https://example.test/about).\n";
const FLOW: &str = "![The release flow](img/flow.png)\n\n![[handbook/start#Start here]]\n";
const URLS: &str = "See [the search](https://example.test/search?q=site%20release&page=2#results \"Search\")\\\nand the logo.\n\n---\n\n![Logo](https://example.test/img/logo%20v2.png?size=small#top)\n";
const FLOW_IMAGE: &[u8] = b"\x89PNG\x00the release flow\x00";

/// A site whose publication holds the four notes and the image of the fixtures.
fn build() -> Result<(Built, String), Failure> {
    let mut start = file(
        "handbook/start.md",
        START.as_bytes(),
        "text/markdown",
        vec![],
        vec![],
    )?;
    start.frontmatter = serde_json::json!({ "title": "Start here" })
        .as_object()
        .cloned();
    let anchor_link = link(
        "handbook/start#Start here",
        "the start",
        LinkForm::Wiki,
        Some("handbook/start.md"),
    );
    let files = vec![
        start,
        file(
            "links.md",
            LINKS.as_bytes(),
            "text/markdown",
            vec![
                anchor_link,
                link("Private", "my notes", LinkForm::Wiki, None),
            ],
            vec![],
        )?,
        file(
            "flow.md",
            FLOW.as_bytes(),
            "text/markdown",
            vec![],
            vec![
                link(
                    "img/flow.png",
                    "The release flow",
                    LinkForm::Markdown,
                    Some("img/flow.png"),
                ),
                link(
                    "handbook/start#Start here",
                    "Start here",
                    LinkForm::Wiki,
                    Some("handbook/start.md"),
                ),
            ],
        )?,
        file("urls.md", URLS.as_bytes(), "text/markdown", vec![], vec![])?,
        file("img/flow.png", FLOW_IMAGE, "image/png", vec![], vec![])?,
    ];
    let texts: BTreeMap<String, String> = [START, LINKS, FLOW, URLS]
        .iter()
        .map(|text| -> Result<(String, String), Failure> {
            let revision = revision_of(text.as_bytes(), "text/markdown")?;
            Ok((revision.digest.hex.as_str().to_owned(), (*text).to_owned()))
        })
        .collect::<Result<_, _>>()?;
    let flow_digest = revision_of(FLOW_IMAGE, "image/png")?
        .digest
        .hex
        .as_str()
        .to_owned();
    let site = SiteInput {
        site_id: SiteId::parse("site-c")?,
        publications: vec![PublicationInput {
            publication: Publication {
                site_id: SiteId::parse("site-c")?,
                publication_id: PublicationId::parse("pub-c")?,
                mode: PublicationMode::All,
                included_folders: Vec::new(),
                excluded_folders: Vec::new(),
                included_tags: Vec::new(),
            },
            connection_id: "connection-c".to_owned(),
            frontmatter_allow: Vec::new(),
            files: &files,
        }],
    };
    Ok((build_release(&site, &texts)?, flow_digest))
}

fn html_of<'a>(built: &'a Built, slug: &str) -> Result<&'a str, Failure> {
    built
        .pages
        .iter()
        .find(|page| page.page.slug.as_str() == slug)
        .map(|page| page.page.html.as_str())
        .ok_or_else(|| format!("the release has no page {slug}").into())
}

#[test]
fn contract_fixture_markup_is_kept() {
    for fixture in [
        WITH_WIKILINK,
        WITH_CALLOUT,
        WITH_EMBED,
        WITH_URLS_AND_BREAKS,
    ] {
        assert_eq!(
            sanitise(fixture),
            fixture,
            "the allowlist keeps the contract markup"
        );
    }
}

#[test]
fn renderer_matches_the_contract_fixtures() -> Outcome {
    let (built, flow_digest) = build()?;
    assert_eq!(html_of(&built, "links")?, WITH_WIKILINK);
    assert_eq!(html_of(&built, "handbook/start")?, WITH_CALLOUT);
    assert_eq!(
        html_of(&built, "flow")?,
        WITH_EMBED.replace(FIXTURE_ASSET, &flow_digest)
    );
    assert_eq!(html_of(&built, "urls")?, WITH_URLS_AND_BREAKS);
    Ok(())
}
