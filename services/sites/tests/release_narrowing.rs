//! SITE-025 tasks 5.67 and 5.69 (review H1 and M1 of 5.63): a publication that a build reads from
//! the release keeps only the notes and attachments that it selected then AND that its current
//! configuration still selects. Selection can only shrink a released publication.

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::bus::{Running, TestBus, TestManifest};
use support::http::{call, owner};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

const CAPABILITIES: [&str; 3] = ["pages.read", "assets.read", "search.publication"];

/// Insert the manual publication `publication` with `mode`, the included folders `included`, the
/// excluded folders `excluded` and the included tags `tags`.
async fn publication(
    fixture: &Fixture,
    publication: &str,
    mode: &str,
    included: &[&str],
    excluded: &[&str],
    tags: &[&str],
) -> Outcome {
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, included_folders, excluded_folders, included_tags, created_at) \
         values ($1, $2, $3, $4, 'manual', $5, $6, $7, now())",
    )
    .bind(SITE)
    .bind(publication)
    .bind(fixture.index.connection())
    .bind(mode)
    .bind(included)
    .bind(excluded)
    .bind(tags)
    .execute(fixture.database.pool())
    .await?;
    Ok(())
}

/// The slugs and the asset digests of the current release document.
async fn released(fixture: &Fixture) -> Result<(Vec<String>, Vec<String>), Failure> {
    let (_, document) =
        ratatoskr_sites::release_read::current_release(&fixture.database.database, SITE)
            .await?
            .ok_or("the site has a release")?;
    let release: ratatoskr_sites_contracts::Release = serde_json::from_slice(&document)?;
    Ok((
        release
            .pages
            .iter()
            .map(|page| page.slug.as_str().to_owned())
            .collect(),
        release
            .assets
            .iter()
            .map(|asset| asset.sha256.as_str().to_owned())
            .collect(),
    ))
}

/// Wait until the worker has built every request.
async fn built(fixture: &Fixture) -> Outcome {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let left: i64 = sqlx::query_scalar("select count(*) from sites.build_requests")
                .fetch_one(fixture.database.pool())
                .await?;
            if left == 0 {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "the requests were not built")?
}

/// PUT `body` as the publication `publication` of the owner.
async fn put(api: &Router, fixture: &Fixture, publication: &str, body: &Value) -> Outcome {
    let answer = call(
        api,
        owner(
            Method::PUT,
            &format!("/v1/sites/{SITE}/publications/{publication}"),
            fixture.owner,
            serde_json::to_vec(body)?,
        )?,
    )
    .await?;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    Ok(())
}

async fn started(fixture: &Fixture) -> Result<(TestBus, Running, Router), Failure> {
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    Ok((bus, running, api))
}

#[tokio::test]
async fn narrowed_publication_drops_its_pages_in_a_build_from_the_release() -> Outcome {
    let fixture = Fixture::new().await?;
    sqlx::query("update sites.publications set release_mode = 'manual'")
        .execute(fixture.database.pool())
        .await?;
    fixture.add_note("team/t.md", "# Team\n\nThe team page.\n")?;
    fixture.add_indexed(
        json!({ "path": "flag/tagged.md", "tags": ["#public"] }),
        "The page with the tag #public.\n",
    )?;
    sqlx::query(
        "update sites.publications set excluded_folders = '{private,flag}' \
         where publication_id = 'pub-r'",
    )
    .execute(fixture.database.pool())
    .await?;
    publication(&fixture, "pub-f", "flagged", &["flag"], &[], &["Public"]).await?;
    let both = TestManifest::of("r.example.test", &["pub-r", "pub-f"], &CAPABILITIES);
    fixture.approve_directly(&both).await?;
    fixture.builder().build(&request(None)).await?;
    let (slugs, _) = released(&fixture).await?;
    assert!(slugs.contains(&"team/t".to_owned()) && slugs.contains(&"flag/tagged".to_owned()));
    let (bus, running, api) = started(&fixture).await?;

    // pub-r now also excludes team/.
    let connection = fixture.index.connection();
    put(
        &api,
        &fixture,
        "pub-r",
        &json!({
            "connection_id": connection, "mode": "all", "release_mode": "manual",
            "included_folders": [], "excluded_folders": ["private", "flag", "team"],
            "included_tags": [], "frontmatter_allow": [],
        }),
    )
    .await?;
    built(&fixture).await?;
    let (slugs, _) = released(&fixture).await?;
    assert!(
        !slugs.contains(&"team/t".to_owned()),
        "a narrowed folder leaves the release: {slugs:?}"
    );
    assert!(
        slugs.contains(&"home".to_owned()),
        "the rest stays: {slugs:?}"
    );

    // pub-f no longer includes the tag Public.
    put(
        &api,
        &fixture,
        "pub-f",
        &json!({
            "connection_id": connection, "mode": "flagged", "release_mode": "manual",
            "included_folders": ["flag"], "excluded_folders": [],
            "included_tags": ["Other"], "frontmatter_allow": [],
        }),
    )
    .await?;
    built(&fixture).await?;
    let (slugs, _) = released(&fixture).await?;
    assert!(
        !slugs.contains(&"flag/tagged".to_owned()),
        "a note of a tag that is no longer included leaves the release: {slugs:?}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
