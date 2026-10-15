//! SITE-025 tasks 5.81, 5.83 and 5.85 (review of 5.79, from the probes of the reviewer): the
//! provenance of a release that a build makes again is written again, a build from the release
//! renders no frontmatter key that the page did not render, and the selection inputs of a note
//! come from the revision that its row stores.

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::bus::{Running, TestBus, TestManifest};
use support::http::{anonymous, call, owner};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

const CAPABILITIES: [&str; 3] = ["pages.read", "assets.read", "search.publication"];

/// Insert the manual publication `publication` in the mode `all` with the included folders
/// `included`.
async fn publication(fixture: &Fixture, publication: &str, included: &[&str]) -> Outcome {
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, included_folders, created_at) \
         values ($1, $2, $3, 'all', 'manual', $4, now())",
    )
    .bind(SITE)
    .bind(publication)
    .bind(fixture.index.connection())
    .bind(included)
    .execute(fixture.database.pool())
    .await?;
    Ok(())
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

/// DELETE the publication `publication` of the owner.
async fn delete(api: &Router, fixture: &Fixture, publication: &str) -> Outcome {
    let removed = call(
        api,
        owner(
            Method::DELETE,
            &format!("/v1/sites/{SITE}/publications/{publication}"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    Ok(())
}

async fn started(fixture: &Fixture) -> Result<(TestBus, Running, Router), Failure> {
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    Ok((bus, running, api))
}

/// The publications of the source rows of `path` in the current release.
async fn publications_of(fixture: &Fixture, path: &str) -> Result<Vec<Vec<String>>, Failure> {
    Ok(sqlx::query_scalar(
        "select s.publication_ids from sites.release_sources s join sites.current_releases c \
         on c.release_id = s.release_id where s.path = $1",
    )
    .bind(path)
    .fetch_all(fixture.database.pool())
    .await?)
}

/// The body of the manual pub-r in the mode `mode` with the excluded folders `excluded`.
fn pub_r(fixture: &Fixture, mode: &str, excluded: &[&str]) -> Value {
    json!({
        "connection_id": fixture.index.connection(), "mode": mode, "release_mode": "manual",
        "included_folders": [], "excluded_folders": excluded,
        "included_tags": [], "frontmatter_allow": [],
    })
}

/// The answer of the page `slug`.
async fn page(api: &Router, slug: &str) -> Result<support::http::Answer, Failure> {
    call(
        api,
        anonymous(
            Method::GET,
            &format!("/v1/sites/{SITE}/content/pages/{slug}"),
        )?,
    )
    .await
}

#[tokio::test]
async fn release_made_again_rewrites_its_provenance() -> Outcome {
    let fixture = Fixture::new().await?;
    sqlx::query("update sites.publications set release_mode = 'manual'")
        .execute(fixture.database.pool())
        .await?;
    fixture.add_note("shared/n.md", "# Shared\n\nA shared note.\n")?;
    publication(&fixture, "pub-a", &["shared"]).await?;
    let both = TestManifest::of("r.example.test", &["pub-r", "pub-a"], &CAPABILITIES);
    fixture.approve_directly(&both).await?;
    let first = fixture.builder().build(&request(None)).await?;
    // A second note makes another release.
    fixture.add_note("shared/m.md", "# M\n\nAnother shared note.\n")?;
    let second = fixture.builder().build(&request(None)).await?;
    assert_ne!(first.release_id, second.release_id);
    let (bus, running, api) = started(&fixture).await?;
    // pub-r excludes shared/: a removal build from the release, the document stays.
    put(
        &api,
        &fixture,
        "pub-r",
        &pub_r(&fixture, "all", &["private", "shared"]),
    )
    .await?;
    built(&fixture).await?;
    assert_eq!(
        publications_of(&fixture, "shared/n.md").await?,
        vec![vec!["pub-a".to_owned()]]
    );

    // m.md leaves the vault; a command build makes the first document again, which is not the
    // current release.
    fixture.remove("shared/m.md")?;
    let third = fixture.builder().build(&request(None)).await?;
    assert_eq!(
        third.release_id, first.release_id,
        "the first document again"
    );
    assert_eq!(
        publications_of(&fixture, "shared/n.md").await?,
        vec![vec!["pub-a".to_owned()]],
        "a release made again gets the provenance of this build"
    );
    let pages: Vec<String> = sqlx::query_scalar(
        "select publication_ids from sites.release_pages where release_id = $1 \
         and slug = 'shared/n'",
    )
    .bind(&third.release_id)
    .fetch_one(fixture.database.pool())
    .await?;
    assert_eq!(pages, ["pub-a"], "and so does its page");

    // pub-r stops excluding shared/: it does not get shared/n back from the release.
    put(
        &api,
        &fixture,
        "pub-r",
        &pub_r(&fixture, "all", &["private"]),
    )
    .await?;
    built(&fixture).await?;
    assert_eq!(
        publications_of(&fixture, "shared/n.md").await?,
        vec![vec!["pub-a".to_owned()]],
        "a build from the release gives pub-r no note that it did not publish"
    );
    delete(&api, &fixture, "pub-a").await?;
    built(&fixture).await?;
    assert_eq!(page(&api, "shared/n").await?.status, StatusCode::NOT_FOUND);

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

/// shared/n.md with the frontmatter field `secret`, which pub-r allows and pub-a does not; pub-a
/// renders the page, because its id comes first.
async fn secret_field() -> Result<Fixture, Failure> {
    let fixture = Fixture::new().await?;
    sqlx::query(
        "update sites.publications set release_mode = 'manual', frontmatter_allow = '{secret}'",
    )
    .execute(fixture.database.pool())
    .await?;
    fixture.add_indexed(
        json!({ "path": "shared/n.md", "frontmatter": { "secret": "hush" } }),
        "---\nsecret: hush\n---\nA shared note.\n",
    )?;
    publication(&fixture, "pub-a", &["shared"]).await?;
    let both = TestManifest::of("r.example.test", &["pub-r", "pub-a"], &CAPABILITIES);
    fixture.approve_directly(&both).await?;
    fixture.builder().build(&request(None)).await?;
    Ok(fixture)
}

#[tokio::test]
async fn deleted_publication_shows_no_new_frontmatter_key() -> Outcome {
    let fixture = secret_field().await?;
    let (bus, running, api) = started(&fixture).await?;
    let before = page(&api, "shared/n").await?;
    assert!(
        !String::from_utf8_lossy(&before.body).contains("hush"),
        "pub-a hides secret"
    );

    delete(&api, &fixture, "pub-a").await?;
    built(&fixture).await?;
    let after = page(&api, "shared/n").await?;
    assert_eq!(after.status, StatusCode::OK, "pub-r keeps the page");
    assert!(
        !String::from_utf8_lossy(&after.body).contains("hush"),
        "a build from the release shows no key that the page did not show"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn widened_allowlist_shows_no_new_key_in_a_build_from_the_release() -> Outcome {
    let fixture = secret_field().await?;
    let (bus, running, api) = started(&fixture).await?;
    // pub-a allows secret now: a PUT, so a removal build from the release.
    put(
        &api,
        &fixture,
        "pub-a",
        &json!({
            "connection_id": fixture.index.connection(), "mode": "all", "release_mode": "manual",
            "included_folders": ["shared"], "excluded_folders": [],
            "included_tags": [], "frontmatter_allow": ["secret"],
        }),
    )
    .await?;
    built(&fixture).await?;
    let after = page(&api, "shared/n").await?;
    assert!(
        !String::from_utf8_lossy(&after.body).contains("hush"),
        "a wider allowlist shows a new key only with a build that reads the notes index"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
