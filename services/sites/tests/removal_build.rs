//! SITE-025 task 5.44 (review L1 to L3 of 5.40): when the effective publications of a site shrink,
//! the service requests a build of the site at once, in each release mode, so the links,
//! backlinks and titles of the removed publication leave the pages that stay.

mod support;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::bus::{TestBus, TestManifest};
use support::http::{anonymous, call, owner, submit};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

const CAPABILITIES: [&str; 3] = ["pages.read", "assets.read", "search.publication"];

/// A site with `pub-r` (manual, the whole vault but `private/`) and `pub-s` (the folder
/// `private/`), both approved, and a first release.
async fn two_publications() -> Result<Fixture, Failure> {
    two_publications_in("manual").await
}

/// As [`two_publications`], with both publications in `release_mode`.
async fn two_publications_in(release_mode: &str) -> Result<Fixture, Failure> {
    let fixture = Fixture::new().await?;
    sqlx::query("update sites.publications set release_mode = $1")
        .bind(release_mode)
        .execute(fixture.database.pool())
        .await?;
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, included_folders, created_at) \
         values ($1, 'pub-s', $2, 'all', $3, '{private}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .bind(release_mode)
    .execute(fixture.database.pool())
    .await?;
    let both = TestManifest::of("r.example.test", &["pub-r", "pub-s"], &CAPABILITIES);
    fixture.approve_directly(&both).await?;
    fixture.builder().build(&request(None)).await?;
    Ok(fixture)
}

async fn requests(fixture: &Fixture) -> Result<i64, Failure> {
    Ok(
        sqlx::query_scalar("select count(*) from sites.build_requests where site_id = $1")
            .bind(SITE)
            .fetch_one(fixture.database.pool())
            .await?,
    )
}

#[tokio::test]
async fn removed_publication_is_built_out_of_the_kept_pages() -> Outcome {
    let fixture = two_publications().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    let home = format!("/v1/sites/{SITE}/content/pages/home");
    let before = call(&api, anonymous(Method::GET, &home)?).await?;
    assert_eq!(before.status, StatusCode::OK);
    let text = String::from_utf8_lossy(&before.body);
    assert!(
        text.contains("private/zq7plannote"),
        "the home page links to the page of pub-s: {text}"
    );
    let first = fixture.release_count().await?;

    let narrower = TestManifest::of("r.example.test", &["pub-r"], &CAPABILITIES);
    assert_eq!(
        submit(&api, fixture.owner, &narrower).await?.status,
        StatusCode::OK
    );
    let built = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if fixture.release_count().await? > first && requests(&fixture).await? == 0 {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        built.is_ok(),
        "the removal requests a build of the manual publication, which makes a new release"
    );

    let after = call(&api, anonymous(Method::GET, &home)?).await?;
    assert_eq!(after.status, StatusCode::OK, "the kept page stays");
    let text = String::from_utf8_lossy(&after.body);
    assert!(
        !text.contains("zq7plannote") && !text.contains("zq7plantitle"),
        "the kept page holds no slug or title of the removed publication: {text}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn automatic_publication_removed_from_the_manifest_requests_a_build() -> Outcome {
    let fixture = two_publications_in("automatic").await?;
    let api = fixture.api().await?;
    assert_eq!(requests(&fixture).await?, 0);

    let narrower = TestManifest::of("r.example.test", &["pub-r"], &CAPABILITIES);
    assert_eq!(
        submit(&api, fixture.owner, &narrower).await?.status,
        StatusCode::OK
    );
    let origins: Vec<String> =
        sqlx::query_scalar("select origin from sites.build_requests where site_id = $1")
            .bind(SITE)
            .fetch_all(fixture.database.pool())
            .await?;
    assert_eq!(
        origins,
        vec!["removal".to_owned()],
        "a publication that leaves the manifest of an automatic site requests one removal build"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn deleted_and_narrowed_publications_request_a_build() -> Outcome {
    let fixture = two_publications().await?;
    let api = fixture.api().await?;
    assert_eq!(requests(&fixture).await?, 0);

    let removed = call(
        &api,
        owner(
            Method::DELETE,
            &format!("/v1/sites/{SITE}/publications/pub-s"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        requests(&fixture).await?,
        1,
        "a deleted publication requests a build"
    );

    sqlx::query("delete from sites.build_requests")
        .execute(fixture.database.pool())
        .await?;
    let narrower = json!({
        "connection_id": fixture.index.connection(),
        "mode": "all",
        "release_mode": "manual",
        "included_folders": [],
        "excluded_folders": ["private", "attachments"],
        "included_tags": [],
        "frontmatter_allow": [],
    });
    let written = call(
        &api,
        owner(
            Method::PUT,
            &format!("/v1/sites/{SITE}/publications/pub-r"),
            fixture.owner,
            serde_json::to_vec(&narrower)?,
        )?,
    )
    .await?;
    assert_eq!(written.status, StatusCode::OK);
    assert_eq!(
        requests(&fixture).await?,
        1,
        "a changed publication with a release requests a build"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn removal_build_keeps_the_released_text() -> Outcome {
    let fixture = two_publications().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    let home = format!("/v1/sites/{SITE}/content/pages/home");
    let first = fixture.release_count().await?;

    // The owner changes a note of the manual publication and does not release it.
    fixture.change(
        "home.md",
        "---\ntitle: Home\npublish: true\n---\n# Home\n\nA zebrafish that the owner did not release.\n",
    )?;
    let narrower = TestManifest::of("r.example.test", &["pub-r"], &CAPABILITIES);
    assert_eq!(
        submit(&api, fixture.owner, &narrower).await?.status,
        StatusCode::OK
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if fixture.release_count().await? > first && requests(&fixture).await? == 0 {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "the removal build made no release")??;

    let page = call(&api, anonymous(Method::GET, &home)?).await?;
    assert_eq!(page.status, StatusCode::OK);
    let text = String::from_utf8_lossy(&page.body);
    assert!(
        text.contains("Start here") && !text.contains("zebrafish"),
        "the removal release holds the released text of the changed note: {text}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn removal_during_a_build_is_not_lost() -> Outcome {
    let fixture = two_publications().await?;
    fixture.add_note("team/t.md", "# Team\n\nThe page of pub-t.\n")?;
    // Only pub-t selects the folder team/.
    sqlx::query("update sites.publications set excluded_folders = '{private,team}' where publication_id = 'pub-r'")
        .execute(fixture.database.pool())
        .await?;
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, included_folders, created_at) \
         values ($1, 'pub-t', $2, 'all', 'manual', '{team}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await?;
    let three = TestManifest::of(
        "r.example.test",
        &["pub-r", "pub-s", "pub-t"],
        &CAPABILITIES,
    );
    fixture.approve_directly(&three).await?;
    fixture.builder().build(&request(None)).await?;

    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, 0).await?;
    let api = fixture.api_of(&running.shared).await?;
    let delete = |publication: &str| {
        owner(
            Method::DELETE,
            &format!("/v1/sites/{SITE}/publications/{publication}"),
            fixture.owner,
            Vec::new(),
        )
    };
    // The removal build of pub-s takes its request and waits at the gate.
    assert_eq!(
        call(&api, delete("pub-s")?).await?.status,
        StatusCode::NO_CONTENT
    );
    running.started(1).await?;
    assert_eq!(
        call(&api, delete("pub-t")?).await?.status,
        StatusCode::NO_CONTENT
    );
    running.open(64);
    tokio::time::timeout(Duration::from_secs(30), async {
        while requests(&fixture).await? > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<(), Failure>(())
    })
    .await
    .map_err(|_| "the requests were not built")??;
    assert_eq!(
        running.builds(),
        2,
        "the removal during the running build makes a second build"
    );
    let (_, document) =
        ratatoskr_sites::release_read::current_release(&fixture.database.database, SITE)
            .await?
            .ok_or("the site has a release")?;
    let document = String::from_utf8_lossy(&document);
    assert!(
        !document.contains("team/t") && !document.contains("pub-t"),
        "the current release has no trace of pub-t: {document}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

/// The slugs of the current release document.
async fn released_slugs(fixture: &Fixture) -> Result<Vec<String>, Failure> {
    let (_, document) =
        ratatoskr_sites::release_read::current_release(&fixture.database.database, SITE)
            .await?
            .ok_or("the site has a release")?;
    let release: ratatoskr_sites_contracts::Release = serde_json::from_slice(&document)?;
    Ok(release
        .pages
        .iter()
        .map(|page| page.slug.as_str().to_owned())
        .collect())
}

/// Wait until the worker has made a release after `first` and has no request left.
async fn next_release(fixture: &Fixture, first: i64) -> Outcome {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if fixture.release_count().await? > first && requests(fixture).await? == 0 {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "no new release")?
}

#[tokio::test]
async fn flagged_notes_stay_published_after_builds_from_the_release() -> Outcome {
    let fixture = two_publications().await?;
    // pub-f, manual and flagged, reads the folder flag/: a note with `publish: "site-r"` and a
    // note with the tag #public, which pub-f includes as `Public`.
    fixture.add_indexed(
        json!({ "path": "flag/string.md", "frontmatter": { "publish": SITE } }),
        "---\npublish: site-r\n---\nThe page that names the site.\n",
    )?;
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
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         release_mode, included_folders, included_tags, created_at) \
         values ($1, 'pub-f', $2, 'flagged', 'manual', '{flag}', '{Public}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await?;
    let three = TestManifest::of(
        "r.example.test",
        &["pub-r", "pub-s", "pub-f"],
        &CAPABILITIES,
    );
    fixture.approve_directly(&three).await?;
    fixture.builder().build(&request(None)).await?;
    let flagged = ["flag/string".to_owned(), "flag/tagged".to_owned()];
    let slugs = released_slugs(&fixture).await?;
    assert!(
        flagged.iter().all(|slug| slugs.contains(slug)),
        "the command build publishes both flagged notes: {slugs:?}"
    );

    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    let first = fixture.release_count().await?;
    let narrower = TestManifest::of("r.example.test", &["pub-r", "pub-f"], &CAPABILITIES);
    assert_eq!(
        submit(&api, fixture.owner, &narrower).await?.status,
        StatusCode::OK
    );
    next_release(&fixture, first).await?;
    let slugs = released_slugs(&fixture).await?;
    assert!(
        flagged.iter().all(|slug| slugs.contains(slug)),
        "the removal build keeps both flagged notes: {slugs:?}"
    );

    // pub-r becomes automatic and a.md changes: the build reads pub-f from the release.
    sqlx::query(
        "update sites.publications set release_mode = 'automatic' where publication_id = 'pub-r'",
    )
    .execute(fixture.database.pool())
    .await?;
    fixture.change("a.md", "---\npublish: true\n---\nThe note a, changed.\n")?;
    let second = fixture.release_count().await?;
    sqlx::query(
        "insert into sites.build_requests (request_id, site_id, origin, operation_id, \
         requested_at) values ($1, $2, 'file_change', null, now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(SITE)
    .execute(fixture.database.pool())
    .await?;
    running.shared.servable.wake(SITE);
    next_release(&fixture, second).await?;
    let slugs = released_slugs(&fixture).await?;
    assert!(
        flagged.iter().all(|slug| slugs.contains(slug)),
        "a build that reads the manual publication from the release keeps both: {slugs:?}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
