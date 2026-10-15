//! SITE-026 tasks 5.1 and 5.2: the sites service rewrites the gateway projection `site.json` of a
//! site on every approval event that it consumes through `ratatoskr_sites_approvals` (workspace
//! spec `site-gateway`, "The gateway projection follows every approval change").

mod support;

use std::time::Duration;

use ratatoskr_sites::consumer::Durable;
use support::bus::{TestBus, approval};
use support::{Failure, Fixture, SITE};

type Outcome = Result<(), Failure>;

const CAPABILITIES: [&str; 3] = ["pages.read", "assets.read", "search.publication"];

/// `site.json` of the fixture site, if it can be read and parsed.
fn projection(fixture: &Fixture) -> Option<serde_json::Value> {
    let bytes = std::fs::read(fixture.deploys_root.path().join(SITE).join("site.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Wait at most `patience` until `site.json` satisfies `wanted`, and give back what it holds then.
async fn projection_when(
    fixture: &Fixture,
    patience: Duration,
    accepts: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    let waited = tokio::time::timeout(patience, async {
        loop {
            if let Some(found) = projection(fixture).filter(&accepts) {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    waited.ok().or_else(|| projection(fixture))
}

/// Publish the approval of the fixture site with `hostname`, or its withdrawal.
async fn publish(fixture: &Fixture, bus: &TestBus, hostname: &str, approved: bool) -> Outcome {
    let digest = Fixture::approved_manifest().digest();
    bus.publish(
        "evt.platform.site_approval.changed.v1",
        approval(
            SITE,
            &digest,
            &CAPABILITIES,
            hostname,
            approved,
            fixture.owner,
        )?,
    )
    .await
}

fn hostnames(projection: &serde_json::Value) -> Vec<String> {
    projection
        .get("hostnames")
        .and_then(serde_json::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn approved(projection: &serde_json::Value) -> bool {
    projection.get("approved") == Some(&serde_json::json!(true))
}

#[tokio::test]
async fn approval_writes_projection_with_hostname() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;

    publish(&fixture, &bus, "r.example.test", true).await?;
    bus.settled(Durable::Approvals).await?;
    let written = projection_when(&fixture, Duration::from_secs(5), approved).await;
    assert_eq!(
        written
            .as_ref()
            .map(|found| (approved(found), hostnames(found))),
        Some((true, vec!["r.example.test".to_owned()])),
        "site.json names the approved hostname: {written:?}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn hostname_change_rewrites_projection() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;

    publish(&fixture, &bus, "old.example", true).await?;
    publish(&fixture, &bus, "new.example", true).await?;
    bus.settled(Durable::Approvals).await?;
    let written = projection_when(&fixture, Duration::from_secs(5), |found| {
        hostnames(found) == ["new.example"]
    })
    .await;
    assert_eq!(
        written.as_ref().map(hostnames),
        Some(vec!["new.example".to_owned()]),
        "site.json lists new.example and not old.example: {written:?}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn withdrawal_rewrites_projection_within_five_seconds() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;

    publish(&fixture, &bus, "r.example.test", true).await?;
    bus.settled(Durable::Approvals).await?;
    projection_when(&fixture, Duration::from_secs(5), approved).await;
    publish(&fixture, &bus, "r.example.test", false).await?;
    let written = projection_when(&fixture, Duration::from_secs(5), |found| {
        !approved(found) && hostnames(found).is_empty()
    })
    .await;
    assert_eq!(
        written
            .as_ref()
            .map(|found| (approved(found), hostnames(found))),
        Some((false, Vec::new())),
        "within five seconds site.json is not approved and lists no hostname: {written:?}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
