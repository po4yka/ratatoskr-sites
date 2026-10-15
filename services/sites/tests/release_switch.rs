//! SITE-025 task 5.20: a release becomes current atomically, and the release event carries no
//! content (workspace spec `site-publications`).

mod support;

use std::collections::BTreeSet;

use ratatoskr_event_envelope::EventEnvelope;
use ratatoskr_sites::release_read::{current_release, page};
use ratatoskr_sites::sites_blobs::SitesBlobs;
use ratatoskr_sites_contracts::{Release, ReleasePublished};
use ratatoskr_sites_release::sha256_hex;
use serde_json::Value;
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

#[tokio::test]
async fn readers_see_one_release_or_the_other() -> Outcome {
    let fixture = Fixture::new().await?;
    let builder = fixture.builder();
    let blobs = SitesBlobs::new(fixture.sites_root.path());
    builder.build(&request(None)).await?;

    // The reader fetches the release document.
    let (fetched, document) = current_release(&fixture.database.database, SITE)
        .await?
        .ok_or("the site has a current release")?;
    let release: Release = serde_json::from_slice(&document)?;

    // A new release becomes current before the reader fetches the pages.
    fixture.change(
        "other.md",
        "---\npublish: true\n---\nThe other page, changed.\n",
    )?;
    let next = builder.build(&request(None)).await?;
    assert_ne!(
        next.release_id, fetched,
        "the second build makes a new release"
    );

    for listed in &release.pages {
        let answer = page(
            &fixture.database.database,
            &blobs,
            SITE,
            &fetched,
            listed.slug.as_str(),
        )
        .await?;
        if let Some(answer) = answer {
            assert_eq!(
                answer.release_id,
                fetched,
                "the page {} belongs to the release that the reader fetched",
                listed.slug.as_str()
            );
            assert_eq!(sha256_hex(&answer.bytes), listed.page_sha256.as_str());
        }
    }
    fixture.cleanup().await
}

#[tokio::test]
async fn published_event_carries_no_content() -> Outcome {
    let fixture = Fixture::new().await?;
    let outcome = fixture.builder().build(&request(None)).await?;
    let document = fixture.document(&outcome.release_id).await?;
    let release: Release = serde_json::from_slice(&document)?;

    let events: Vec<Value> = sqlx::query_scalar(
        "select payload from sites.outbox where subject = 'evt.sites.release.published.v1'",
    )
    .fetch_all(fixture.database.pool())
    .await?;
    assert_eq!(events.len(), 1, "the new release is announced once");
    let bytes = serde_json::to_vec(events.first().ok_or("an event")?)?;

    let envelope: EventEnvelope = serde_json::from_slice(&bytes)?;
    let payload: ReleasePublished = envelope.payload_as()?;
    assert_eq!(payload.site_id.as_str(), SITE);
    assert_eq!(payload.release_id.as_str(), outcome.release_id);
    assert_eq!(usize::try_from(payload.page_count)?, release.pages.len());
    let members: BTreeSet<&str> = envelope.payload.keys().map(String::as_str).collect();
    assert_eq!(
        members,
        BTreeSet::from([
            "page_count",
            "publication_ids",
            "published_at",
            "release_id",
            "site_id"
        ]),
        "the payload has only the members of the contract"
    );

    let text = String::from_utf8(bytes)?;
    let files = fixture.index.files_now()?;
    let names = release
        .pages
        .iter()
        .flat_map(|page| {
            [
                page.slug.as_str().to_owned(),
                page.title.as_str().to_owned(),
            ]
        })
        .chain(files.into_iter().map(|file| file.path))
        .filter(|name| name.len() >= 5);
    for name in names {
        assert!(
            !text.contains(&name),
            "the event holds no title, slug or path: {name}"
        );
    }
    fixture.cleanup().await
}

#[tokio::test]
async fn failed_switch_changes_nothing() -> Outcome {
    let fixture = Fixture::new().await?;
    let builder = fixture.builder();
    let first = builder.build(&request(None)).await?;

    fixture.change(
        "other.md",
        "---\npublish: true\n---\nThe other page, changed.\n",
    )?;
    // The outbox cannot take the release event, so the transaction of the switch fails.
    sqlx::query("alter table sites.outbox rename to outbox_gone")
        .execute(fixture.database.pool())
        .await?;
    let second = builder.build(&request(None)).await;
    assert!(second.is_err(), "the build fails");

    let (current, _) = current_release(&fixture.database.database, SITE)
        .await?
        .ok_or("the site has a current release")?;
    assert_eq!(
        current, first.release_id,
        "the current release did not change"
    );
    assert_eq!(
        fixture.release_count().await?,
        1,
        "no row of the second release stays"
    );
    let pages: i64 =
        sqlx::query_scalar("select count(*) from sites.release_pages where release_id <> $1")
            .bind(&first.release_id)
            .fetch_one(fixture.database.pool())
            .await?;
    assert_eq!(pages, 0, "no page row of the second release stays");
    fixture.cleanup().await
}
