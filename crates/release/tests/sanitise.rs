//! SITE-025 task 5.16: rendered HTML is sanitised (workspace spec `site-release-privacy`,
//! requirement "Rendered HTML is sanitised").

use std::collections::BTreeMap;

use ratatoskr_sites_contracts::{Page, PublicationId, SiteId};
use ratatoskr_sites_publications::test_support::revision_of;
use ratatoskr_sites_publications::{BuildError, IndexedFile, Publication, PublicationMode};
use ratatoskr_sites_release::sanitise::sanitise;
use ratatoskr_sites_release::{Built, PublicationInput, SiteInput, build_release};
use serde_json::Value;

type Outcome = Result<(), Box<dyn std::error::Error>>;
type Failure = Box<dyn std::error::Error>;

/// The build of a site whose only note is `note.md` with the text `text`.
fn build_of(text: &str) -> Result<Result<Built, BuildError>, Failure> {
    let file = IndexedFile {
        path: "note.md".to_owned(),
        revision: revision_of(text.as_bytes(), "text/markdown")?,
        frontmatter: None,
        tags: Vec::new(),
        headings: Vec::new(),
        blocks: Vec::new(),
        links: Vec::new(),
        embeds: Vec::new(),
    };
    let texts = BTreeMap::from([(
        file.revision.digest.hex.as_str().to_owned(),
        text.to_owned(),
    )]);
    let files = [file];
    let site = SiteInput {
        site_id: SiteId::parse("site-s")?,
        publications: vec![PublicationInput {
            publication: Publication {
                site_id: SiteId::parse("site-s")?,
                publication_id: PublicationId::parse("pub-s")?,
                mode: PublicationMode::All,
                included_folders: Vec::new(),
                excluded_folders: Vec::new(),
                included_tags: Vec::new(),
            },
            connection_id: "connection-s".to_owned(),
            frontmatter_allow: Vec::new(),
            files: &files,
        }],
    };
    Ok(build_release(&site, &texts))
}

/// The one page of a site whose only note is `note.md` with the text `text`.
fn page_of(text: &str) -> Result<Page, Failure> {
    build_of(text)??
        .pages
        .into_iter()
        .next()
        .map(|built| built.page)
        .ok_or_else(|| "the release has one page".into())
}

#[test]
fn script_and_javascript_urls_are_removed() -> Outcome {
    let page = page_of(concat!(
        "Before.\n\n",
        "<script>alert(1)</script>\n\n",
        "A <a href=\"javascript:x\">raw link</a> and [a Markdown link](javascript:y).\n\n",
        "<img src=\"https://example.test/a.png\" onerror=\"alert(2)\">\n\n",
        "<iframe src=\"https://example.test\"></iframe><style>p{}</style>",
        "<form action=\"https://example.test\"><input></form>\n\n",
        "After.\n",
    ))?;
    let html = page.html.to_ascii_lowercase();
    assert!(
        !html.contains("<script"),
        "the HTML holds no script element: {html}"
    );
    assert!(
        !html.contains("javascript:"),
        "the HTML holds no javascript: URL: {html}"
    );
    assert!(
        !html.contains("onerror"),
        "the HTML holds no on* attribute: {html}"
    );
    for element in ["<iframe", "<style", "<form", "<object", "<embed"] {
        assert!(
            !html.contains(element),
            "the HTML holds no {element}: {html}"
        );
    }
    assert!(
        html.contains("before.") && html.contains("after."),
        "the text around the markup stays: {html}"
    );
    let ast = serde_json::to_string(&page.ast)?;
    assert!(
        !ast.contains("script") && !ast.contains("javascript:"),
        "the page tree holds no raw HTML: {ast}"
    );
    Ok(())
}

#[test]
fn dataview_block_is_rendered_as_code() -> Outcome {
    let page = page_of("```dataview\nTABLE file.name FROM \"blog\"\n```\n")?;
    let ast = serde_json::to_value(&page.ast)?;
    let block = ast.pointer("/children/0").ok_or("the page has one block")?;
    assert_eq!(block.get("type"), Some(&Value::from("code")), "{ast}");
    assert_eq!(block.get("lang"), Some(&Value::from("dataview")), "{ast}");
    assert_eq!(
        block.get("value"),
        Some(&Value::from("TABLE file.name FROM \"blog\"\n")),
        "the query is kept as text and is not run: {ast}"
    );
    assert!(
        page.html
            .contains("<pre><code class=\"language-dataview\">TABLE file.name FROM &quot;blog&quot;\n</code></pre>"),
        "the HTML shows the query as code: {}",
        page.html
    );
    Ok(())
}

/// How often `<name` opens and `</name>` closes the element `name` in `html`.
fn tag_counts(html: &str, name: &str) -> (usize, usize) {
    let opening = format!("<{name}");
    let opens = html
        .match_indices(&opening)
        .filter(|(at, _)| {
            html.get(at + opening.len()..)
                .and_then(|rest| rest.chars().next())
                .is_some_and(|next| next == '>' || next == ' ' || next == '/')
        })
        .count();
    (opens, html.matches(&format!("</{name}>")).count())
}

/// The non-void elements of the allowlist.
const NON_VOID: [&str; 39] = [
    "a",
    "abbr",
    "b",
    "blockquote",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "i",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "s",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "u",
];

fn assert_balanced(html: &str) {
    for name in NON_VOID {
        let (opens, closes) = tag_counts(html, name);
        assert_eq!(opens, closes, "<{name}> is balanced in {html}");
    }
}

#[test]
fn self_closing_non_void_elements_are_balanced() {
    for payload in [
        "<a href=\"https://example.test\" />text",
        "<div/>text",
        "<table/>text",
        "<p/>text<b/>bold",
    ] {
        let once = sanitise(payload);
        assert_balanced(&once);
        assert_eq!(
            sanitise(&once),
            once,
            "sanitising twice changes nothing: {payload}"
        );
    }
}

#[test]
fn void_and_raw_elements_leave_no_text() {
    let embed = sanitise("<embed src=\"https://example.test/x\">after");
    assert!(
        !embed.contains("<embed") && embed.contains("after"),
        "{embed}"
    );
    for (payload, hidden) in [
        ("<script/>alert(1)</script>after", "alert"),
        ("<style/>p{color:red}</style>after", "color"),
    ] {
        let clean = sanitise(payload);
        assert!(
            !clean.contains(hidden) && clean.contains("after"),
            "the content of a self-closed {hidden} element is removed: {clean}"
        );
    }
}

#[test]
fn deep_nesting_is_bounded_and_fast() {
    let deep = sanitise(&("<b>".repeat(300) + "x"));
    let (opens, _) = tag_counts(&deep, "b");
    assert!(
        opens <= 256,
        "at most 256 elements are open at once: {opens}"
    );
    assert!(deep.contains('x'), "the text inside stays");

    let payload = "<b>".repeat(50_000) + &"</i>".repeat(50_000);
    let started = std::time::Instant::now();
    let _ = sanitise(&payload);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "50 000 open elements and 50 000 stray end tags take {elapsed:?}"
    );
}

#[test]
fn html_over_one_mebibyte_fails_the_build() -> Outcome {
    let built = build_of(&"word ".repeat(230_000))?;
    assert_eq!(
        built.as_ref().err().map(|error| error.class),
        Some("sites.page_too_large"),
        "a page body over 1 MiB of HTML makes no release"
    );
    Ok(())
}
