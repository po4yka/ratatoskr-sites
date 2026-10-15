//! SITE-025 task 5.24: a disconnected vault suspends its publications, and a withdrawn site
//! serves exactly what a site that never existed serves (workspace spec `site-publications`).
//! Against a real `JetStream`.

mod support;

use std::time::Duration;

use ratatoskr_sites::consumer::Durable;
use ratatoskr_sites::release_read::current_release;
use support::bus::{TestBus, approval, build_command, disconnection, report_of};
use support::http::assert_same_as_unknown;
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// How long a test waits for the build worker.
const PATIENCE: Duration = Duration::from_secs(30);

/// Publish a build command for the operation `operation_id`, and wait for its report.
async fn command(
    fixture: &Fixture,
    bus: &TestBus,
    operation_id: Uuid,
) -> Result<serde_json::Value, Failure> {
    bus.publish(
        "cmd.sites.release.build_requested.v1",
        build_command(SITE, fixture.owner, operation_id)?,
    )
    .await?;
    let report = tokio::time::timeout(PATIENCE, async {
        loop {
            if let Some(report) = report_of(fixture, operation_id).await? {
                return Ok::<_, Failure>(report);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "the build reported nothing")??;
    Ok(report)
}

#[tokio::test]
async fn suspended_publication_does_not_build() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4);
    let first = command(&fixture, &bus, Uuid::now_v7()).await?;
    assert_eq!(first.get("status"), Some(&serde_json::json!("succeeded")));
    let (before, _) = current_release(&fixture.database.database, SITE)
        .await?
        .ok_or("the first build makes a release")?;

    bus.publish(
        "evt.notes.vault.disconnected.v1",
        disconnection(fixture.index.connection(), fixture.owner)?,
    )
    .await?;
    bus.settled(Durable::VaultDisconnections).await?;
    fixture.change(
        "other.md",
        "---\npublish: true\n---\nChanged after the disconnection.\n",
    )?;

    let report = command(&fixture, &bus, Uuid::now_v7()).await?;
    assert_eq!(
        report.pointer("/error/code"),
        Some(&serde_json::json!("sites.publication_suspended")),
        "a build of a suspended publication fails: {report}"
    );
    let (after, _) = current_release(&fixture.database.database, SITE)
        .await?
        .ok_or("the site keeps its release")?;
    assert_eq!(after, before, "the current release does not change");

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn withdrawn_site_is_indistinguishable_from_unknown() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4);
    command(&fixture, &bus, Uuid::now_v7()).await?;

    let digest = "a".repeat(64);
    let capabilities = ["pages.read", "assets.read", "search.publication"];
    for approved in [true, false] {
        bus.publish(
            "evt.platform.site_approval.changed.v1",
            approval(
                SITE,
                &digest,
                &capabilities,
                "r.example.test",
                approved,
                fixture.owner,
            )?,
        )
        .await?;
    }
    bus.settled(Durable::Approvals).await?;

    assert_same_as_unknown(&fixture.api().await?, SITE, "never-was").await?;

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
