//! SITE-025 task 5.26: a submitted manifest grants nothing, the sites service holds no approval,
//! a withdrawal from Platform stops the site, and Edge reads the active manifest on a loopback
//! route (workspace spec `site-manifest`). Against a real `JetStream`.

mod support;

use std::net::SocketAddr;

use axum::http::{Method, StatusCode};
use ratatoskr_sites_contracts::SiteCapability;
use support::bus::{TestBus, TestManifest, approve, requested_build};
use support::http::{call, call_from, owner, submit};
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

const PUBLIC: [SiteCapability; 4] = [
    SiteCapability::PagesRead,
    SiteCapability::AssetsRead,
    SiteCapability::SearchPublication,
    SiteCapability::LiveRelease,
];

fn manifest() -> TestManifest {
    TestManifest::of(
        "r.example.test",
        &["pub-r"],
        &["pages.read", "assets.read", "search.publication"],
    )
}

/// The approval state of the fixture site in `GET /v1/sites`.
async fn approval_state(router: &axum::Router, user: Uuid) -> Result<serde_json::Value, Failure> {
    let answer = call(router, owner(Method::GET, "/v1/sites", user, Vec::new())?).await?;
    assert_eq!(answer.status, StatusCode::OK, "the site list answers");
    let list = answer.json()?;
    let site = list
        .get("sites")
        .and_then(serde_json::Value::as_array)
        .and_then(|sites| {
            sites
                .iter()
                .find(|site| site.get("site_id") == Some(&serde_json::json!(SITE)))
        })
        .ok_or("the list holds the site")?;
    Ok(site.clone())
}

#[tokio::test]
async fn submitted_manifest_alone_serves_nothing() -> Outcome {
    let fixture = Fixture::unapproved().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;

    let manifest = manifest();
    let answer = submit(&api, fixture.owner, &manifest).await?;
    assert_eq!(answer.status, StatusCode::OK, "the submission is stored");
    requested_build(&fixture, &bus, Uuid::now_v7()).await?;

    let state = approval_state(&api, fixture.owner).await?;
    assert_eq!(
        state.get("submitted_manifest_sha256"),
        Some(&serde_json::json!(manifest.digest())),
        "the submission is the active manifest"
    );
    assert_eq!(
        state.get("approval_state"),
        Some(&serde_json::json!("unapproved"))
    );
    for capability in PUBLIC {
        assert_eq!(
            running.shared.servable.serving(SITE, capability),
            None,
            "an unapproved site serves nothing for {capability:?}"
        );
    }

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn sites_service_has_no_approval_route() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let unknown = call(
        &api,
        owner(Method::GET, "/v1/no-such-route", fixture.owner, Vec::new())?,
    )
    .await?;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    for (method, path) in [
        (Method::POST, format!("/internal/sites/{SITE}/approval")),
        (Method::POST, format!("/v1/sites/{SITE}/manifest/approval")),
        (
            Method::DELETE,
            format!("/v1/sites/{SITE}/manifest/approval"),
        ),
    ] {
        let answer = call(
            &api,
            owner(method.clone(), &path, fixture.owner, b"{}".to_vec())?,
        )
        .await?;
        assert_eq!(
            answer.status, unknown.status,
            "{method} {path} is an unknown route"
        );
        assert_eq!(
            answer.body, unknown.body,
            "{method} {path} is an unknown route"
        );
    }

    let tables: Vec<String> = sqlx::query_scalar(
        "select table_name::text from information_schema.tables \
         where table_schema = 'sites' and table_name like '%approv%' order by 1",
    )
    .fetch_all(fixture.database.pool())
    .await?;
    assert_eq!(
        tables,
        vec!["approval_projection".to_owned()],
        "the read projection is the only table of an approval"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn withdrawal_event_stops_the_site() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;
    let manifest = manifest();
    submit(&api, fixture.owner, &manifest).await?;
    approve(&bus, &fixture, &manifest, true).await?;
    requested_build(&fixture, &bus, Uuid::now_v7()).await?;

    assert!(
        running
            .shared
            .servable
            .serving(SITE, SiteCapability::PagesRead)
            .is_some(),
        "an approved site with a release serves its pages"
    );
    let state = approval_state(&api, fixture.owner).await?;
    assert_eq!(
        state.get("approval_state"),
        Some(&serde_json::json!("approved"))
    );

    approve(&bus, &fixture, &manifest, false).await?;
    for capability in PUBLIC {
        assert_eq!(
            running.shared.servable.serving(SITE, capability),
            None,
            "a withdrawn site serves nothing for {capability:?}"
        );
    }
    let state = approval_state(&api, fixture.owner).await?;
    assert_eq!(
        state.get("approval_state"),
        Some(&serde_json::json!("withdrawn"))
    );
    assert_eq!(
        state.get("approved_manifest_sha256"),
        Some(&serde_json::json!(manifest.digest()))
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn internal_manifest_route_serves_loopback_only() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let manifest = manifest();
    submit(&api, fixture.owner, &manifest).await?;
    let path = format!("/internal/sites/{SITE}/manifest");

    let answer = call(&api, owner(Method::GET, &path, fixture.owner, Vec::new())?).await?;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "a loopback peer reads the manifest"
    );
    assert_eq!(
        answer.json()?,
        serde_json::json!({
            "site_id": SITE,
            "manifest_sha256": manifest.digest(),
            "hostname": "r.example.test",
            "publications": ["pub-r"],
            "capabilities": ["pages.read", "assets.read", "search.publication"],
        })
    );

    let remote = SocketAddr::from(([192, 0, 2, 10], 5000));
    let refused = call_from(
        &api,
        owner(Method::GET, &path, fixture.owner, Vec::new())?,
        remote,
    )
    .await?;
    assert_eq!(
        refused.status,
        StatusCode::FORBIDDEN,
        "another peer is refused"
    );
    assert_eq!(refused.code().as_deref(), Some("sites.request.forbidden"));
    assert!(
        !String::from_utf8_lossy(&refused.body).contains("r.example.test"),
        "the refusal holds no manifest data"
    );
    fixture.cleanup().await
}
