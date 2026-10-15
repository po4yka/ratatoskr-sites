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

#[tokio::test]
async fn every_source_row_names_its_publications() -> Outcome {
    let fixture = Fixture::new().await?;
    let built = fixture.builder().build(&request(None)).await?;
    let unnamed: i64 = sqlx::query_scalar(
        "select count(*) from sites.release_sources where cardinality(publication_ids) = 0",
    )
    .fetch_one(fixture.database.pool())
    .await?;
    assert_eq!(
        unnamed, 0,
        "every note and attachment names its publications"
    );

    let inserted = sqlx::query(
        "insert into sites.release_sources (release_id, connection_id, path, file, \
         publication_ids) values ($1, $2, 'stray.md', '{}'::jsonb, '{}')",
    )
    .bind(&built.release_id)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await;
    assert!(
        inserted.is_err(),
        "the schema refuses a source row without publications"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn stored_frontmatter_holds_no_publish() -> Outcome {
    let fixture = Fixture::new().await?;
    sqlx::query("update sites.publications set frontmatter_allow = '{publish}'")
        .execute(fixture.database.pool())
        .await?;
    fixture.builder().build(&request(None)).await?;
    let with_publish: i64 = sqlx::query_scalar(
        "select count(*) from sites.release_sources where file -> 'frontmatter' ? 'publish'",
    )
    .fetch_one(fixture.database.pool())
    .await?;
    assert_eq!(
        with_publish, 0,
        "the stored frontmatter holds no publish value, also when the allowlist names it"
    );
    fixture.cleanup().await
}
