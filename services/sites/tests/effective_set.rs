//! SITE-025 task 5.28: the effective set of a site is the approved manifest intersected with the
//! active one (workspace spec `site-manifest`). An addition waits for an approval; a removal stops
//! at once. Against a real `JetStream`.

mod support;

use std::collections::BTreeSet;

use ratatoskr_sites::effective::effective_set;
use ratatoskr_sites::release_read::current_release;
use ratatoskr_sites_contracts::{Release, SiteCapability};
use support::bus::{Running, TestBus, TestManifest, approve, requested_build};
use support::http::submit;
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// A started service with `manifest` submitted and approved and a first release.
async fn approved(
    manifest: &TestManifest,
) -> Result<(Fixture, TestBus, Running, axum::Router), Failure> {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    submit(&api, fixture.owner, manifest).await?;
    approve(&bus, &fixture, manifest, true).await?;
    requested_build(&fixture, &bus, Uuid::now_v7()).await?;
    Ok((fixture, bus, running, api))
}

fn serves(running: &Running, capability: SiteCapability) -> bool {
    running.shared.servable.serving(SITE, capability).is_some()
}

#[tokio::test]
async fn added_capability_waits_for_approval() -> Outcome {
    let first = TestManifest::of("r.example.test", &["pub-r"], &["pages.read", "assets.read"]);
    let (fixture, bus, running, api) = approved(&first).await?;
    assert!(
        serves(&running, SiteCapability::PagesRead),
        "the approved site serves"
    );
    assert!(!serves(&running, SiteCapability::SearchPublication));

    let wider = TestManifest::of(
        "r.example.test",
        &["pub-r"],
        &["pages.read", "assets.read", "search.publication"],
    );
    submit(&api, fixture.owner, &wider).await?;
    assert!(
        !serves(&running, SiteCapability::SearchPublication),
        "an added capability waits for an approval"
    );
    approve(&bus, &fixture, &wider, true).await?;
    assert!(
        serves(&running, SiteCapability::SearchPublication),
        "the approval of the new digest adds the capability"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn removed_capability_stops_without_approval() -> Outcome {
    let first = TestManifest::of(
        "r.example.test",
        &["pub-r"],
        &["pages.read", "assets.read", "search.publication"],
    );
    let (fixture, bus, running, api) = approved(&first).await?;
    assert!(serves(&running, SiteCapability::SearchPublication));

    let narrower = TestManifest::of("r.example.test", &["pub-r"], &["pages.read", "assets.read"]);
    submit(&api, fixture.owner, &narrower).await?;
    assert!(
        !serves(&running, SiteCapability::SearchPublication),
        "a removed capability stops at once"
    );
    assert!(
        serves(&running, SiteCapability::PagesRead),
        "the rest stays"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn new_hostname_and_publication_wait_for_approval() -> Outcome {
    let first = TestManifest::of("r.example.test", &["pub-r"], &["pages.read"]);
    let (fixture, bus, running, api) = approved(&first).await?;
    // A second publication over the same connection, which selects the private folder.
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         included_folders, created_at) values ($1, 'pub-s', $2, 'all', '{private}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await?;

    let moved = TestManifest::of("s.example.test", &["pub-r", "pub-s"], &["pages.read"]);
    submit(&api, fixture.owner, &moved).await?;
    let effective = effective_set(&fixture.database.database, SITE).await?;
    assert_eq!(
        effective.hostname.as_deref(),
        Some("r.example.test"),
        "the effective hostname stays the approved one"
    );
    assert_eq!(effective.publications, BTreeSet::from(["pub-r".to_owned()]));

    requested_build(&fixture, &bus, Uuid::now_v7()).await?;
    let release = release_of(&fixture).await?;
    let publications: Vec<&str> = release
        .publication_ids
        .iter()
        .map(ratatoskr_sites_contracts::PublicationId::as_str)
        .collect();
    assert_eq!(
        publications,
        ["pub-r"],
        "the added publication is not in the release"
    );

    approve(&bus, &fixture, &moved, true).await?;
    requested_build(&fixture, &bus, Uuid::now_v7()).await?;
    let release = release_of(&fixture).await?;
    assert_eq!(release.publication_ids.len(), 2, "after the approval it is");
    let effective = effective_set(&fixture.database.database, SITE).await?;
    assert_eq!(effective.hostname.as_deref(), Some("s.example.test"));

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

async fn release_of(fixture: &Fixture) -> Result<Release, Failure> {
    let (_, document) = current_release(&fixture.database.database, SITE)
        .await?
        .ok_or("the site has a release")?;
    Ok(serde_json::from_slice(&document)?)
}

#[tokio::test]
async fn unrequested_public_capability_is_not_found() -> Outcome {
    let manifest = TestManifest::of("r.example.test", &["pub-r"], &["pages.read", "assets.read"]);
    let (fixture, bus, running, _) = approved(&manifest).await?;
    assert!(serves(&running, SiteCapability::PagesRead));
    for capability in [
        SiteCapability::SearchPublication,
        SiteCapability::LiveRelease,
    ] {
        assert_eq!(
            running.shared.servable.serving(SITE, capability),
            running.shared.servable.serving("never-was", capability),
            "{capability:?} is absent as for a site that does not exist"
        );
    }

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
