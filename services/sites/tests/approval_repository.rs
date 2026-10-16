//! SITE-028: the approval consumer retains the authoritative repository binding.

mod support;

use ratatoskr_sites::consumer::Durable;
use support::bus::{TestBus, approval};
use support::http::call;
use support::{Failure, Fixture, SITE};

#[tokio::test]
async fn approval_retains_repository_name_and_immutable_id() -> Result<(), Failure> {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, 0).await?;
    let binding = serde_json::json!({"repository": "owner/docs-site", "repository_id": "12345"});
    let bytes = approval(
        SITE,
        &Fixture::approved_manifest().digest(),
        &[],
        "r.example.test",
        true,
        fixture.owner,
    )?;
    let mut event: serde_json::Value = serde_json::from_slice(&bytes)?;
    event["payload"]["github_repository"] = binding.clone();
    bus.publish(
        "evt.platform.site_approval.changed.v1",
        serde_json::to_vec(&event)?,
    )
    .await?;
    bus.settled(Durable::Approvals).await?;
    // to_jsonb also works on the old schema: the RED is the absent binding, not a SQL error.
    let stored: Option<serde_json::Value> = sqlx::query_scalar(
        "select to_jsonb(a)->'github_repository' from sites.approval_projection a where site_id = $1",
    ).bind(SITE).fetch_one(fixture.database.pool()).await?;
    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await?;
    assert_eq!(
        stored,
        Some(binding),
        "the verified approval event identity must survive projection"
    );
    Ok(())
}

#[tokio::test]
async fn manifest_lookup_preserves_repository_binding() -> Result<(), Failure> {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let mut bytes = Fixture::approved_manifest().bytes();
    bytes.extend_from_slice(
        b"\n[github_repository]\nrepository = \"owner/docs-site\"\nrepository_id = \"12345\"\n",
    );
    let submitted = call(
        &api,
        support::http::owner(
            axum::http::Method::PUT,
            &format!("/v1/sites/{SITE}/manifest"),
            fixture.owner,
            bytes,
        )?,
    )
    .await?;
    assert_eq!(
        submitted.status,
        axum::http::StatusCode::OK,
        "{:?}",
        submitted.code()
    );
    let internal = call(
        &api,
        support::http::owner(
            axum::http::Method::GET,
            &format!("/internal/sites/{SITE}/manifest"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(internal.status, axum::http::StatusCode::OK);
    let submitted = submitted.json()?;
    let internal = internal.json()?;
    fixture.cleanup().await?;
    let binding = serde_json::json!({"repository": "owner/docs-site", "repository_id": "12345"});
    assert_eq!(submitted.get("github_repository"), Some(&binding));
    assert_eq!(internal.get("github_repository"), Some(&binding));
    Ok(())
}
