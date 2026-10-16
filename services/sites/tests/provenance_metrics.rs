//! The operator sees bounded outcome labels, never proof data or repository identities.
mod support;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt as _;
use ratatoskr_sites::admin::{RuntimeState, admin_router};
use ratatoskr_sites_contracts::{DeployRequest, SiteId};
use ratatoskr_sites_deploys::{DeployError, DeployStore, Limits};
use std::sync::Arc;
use support::{Failure, Fixture, SITE};
use tower::ServiceExt as _;

#[tokio::test]
async fn verifier_outcomes_are_exposed_without_payload_labels() -> Result<(), Failure> {
    let fixture = Fixture::new().await?;
    let store = DeployStore::new(
        fixture.database.database.clone(),
        fixture.deploys_root.path().to_owned(),
        Limits {
            max_bundle_bytes: 1024,
            retention: 2,
        },
    );
    let site = SiteId::parse(SITE)?;
    let request: DeployRequest = serde_json::from_value(serde_json::json!({
        "commit": "a".repeat(40), "artifact_sha256": "b".repeat(64),
        "attestation_bundle": {"untrusted": "do-not-log-proof"},
    }))?;
    assert!(matches!(
        store.receive_request(&site, request.clone()).await,
        Err(DeployError::VerifierUnavailable)
    ));
    assert!(matches!(
        store.activate(&site, &request.commit).await,
        Err(DeployError::VerifierUnavailable)
    ));
    let metrics = store.operation_metrics();
    assert!(
        metrics.contains("ratatoskr_sites_bundle_verifications_total{outcome=\"unavailable\"} 1")
    );
    assert!(
        metrics.contains("ratatoskr_sites_bundle_activations_total{outcome=\"unavailable\"} 1")
    );
    assert!(!metrics.contains("do-not-log-proof") && !metrics.contains(SITE));
    let runtime =
        Arc::new(RuntimeState::new(fixture.database.database.clone()).with_deploy_metrics(store));
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let response = admin_router(runtime, recorder.handle())
        .oneshot(Request::builder().uri("/metrics").body(Body::empty())?)
        .await?;
    let body = String::from_utf8(response.into_body().collect().await?.to_bytes().to_vec())?;
    assert!(body.contains("ratatoskr_sites_bundle_verifications_total{outcome=\"unavailable\"} 1"));
    fixture.cleanup().await
}
