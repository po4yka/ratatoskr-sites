//! SITE-025 task 5.53 (review H1 of 5.52): `sites.release_sources` holds nothing that comes from a
//! file that the release does not publish (`schema.sql`: no column holds the path of an
//! unpublished file; workspace spec `site-release-privacy`), and nothing of a `%%` comment of a
//! published note (task 5.64).

mod support;

use support::{Failure, Fixture, request};

type Outcome = Result<(), Failure>;

#[tokio::test]
async fn sources_hold_nothing_of_an_unselected_file() -> Outcome {
    let fixture = Fixture::new().await?;
    // A copy of the published image, with the same bytes, in the excluded folder and in the
    // configuration folder.
    let published = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/privacy-vault/vault/attachments/2026/passport-scan-blurred.png"
    ))?;
    fixture.add_file("private/zq7copy.png", &published, "image/png")?;
    fixture.add_file(".obsidian/zq7cache.png", &published, "image/png")?;
    // A published note whose links are only inside a comment: an alias and a heading subpath.
    fixture.add_indexed(
        serde_json::json!({
            "path": "commented.md",
            "frontmatter": { "publish": true },
            "links": [
                { "link": "other", "display_text": "zq7alias", "form": "wiki",
                  "target": "other.md" },
                { "link": "other#zq7commentheading", "display_text": "other > zq7commentheading",
                  "form": "wiki", "target": "other.md" },
            ],
        }),
        "---\npublish: true\n---\nVisible text.\n\n%% See [[other|zq7alias]] and \
         [[other#zq7commentheading]]. %%\n",
    )?;
    fixture.builder().build(&request(None)).await?;

    let rows: Vec<String> = sqlx::query_scalar(
        "select row_to_json(s)::text from sites.release_sources s order by s.path",
    )
    .fetch_all(fixture.database.pool())
    .await?;
    assert!(!rows.is_empty(), "the release has sources");
    for row in &rows {
        for canary in [
            "zq7",
            "img/plan.png",
            "owner_email",
            "owner-marker",
            ".obsidian",
        ] {
            assert!(
                !row.contains(canary),
                "a source row holds {canary} of a file or a field that the release does not \
                 publish: {row}"
            );
        }
    }
    fixture.cleanup().await
}
