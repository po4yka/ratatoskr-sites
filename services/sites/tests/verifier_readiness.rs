//! A live process with no verifier must not report that deployment is ready.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ratatoskr_sites::admin::{RuntimeState, admin_router};
use ratatoskr_sites_persistence::test_support::TestDatabase;
use std::sync::Arc;
use tower::ServiceExt as _;

#[tokio::test]
async fn missing_verifier_keeps_readiness_closed() -> Result<(), Box<dyn std::error::Error>> {
    let database = TestDatabase::create().await?;
    let runtime = Arc::new(RuntimeState::new(database.database.clone()));
    runtime.mark_started();
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let router = admin_router(runtime, recorder.handle());
    let answer = router
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        answer.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "missing verifier was ready"
    );
    database.cleanup().await?;
    Ok(())
}
