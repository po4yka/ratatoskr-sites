//! SITE-026, local tasks 4.13 and 4.14: the deploy routes of the domain API, which Edge forwards in
//! the route class `deploy` (Platform SITE-026 group 8): `POST /v1/sites/{site}/deploys` with the
//! digest and untrusted proof as its JSON body, and `POST /v1/sites/{site}/deploys/{commit}/activate`.

mod support;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use support::http::call;
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

const COMMIT: &str = "3f6c2a9e8b1d4c7f0a5e2b9d6c3f8a1e4b7d0c2f";

/// A client proof object is untrusted; no verifier is configured in this route fixture.
fn bundle() -> Result<Vec<u8>, Failure> {
    Ok(serde_json::to_vec(&serde_json::json!({
        "commit": COMMIT, "artifact_sha256": "a".repeat(64),
        "attestation_bundle": {"untrusted": true},
    }))?)
}

/// A deploy request with the claims that Edge mints for the user `user`, and for a `site_deployer`
/// device of `deployer_site` when one is given.
fn deploy_request(
    path: &str,
    user: Uuid,
    deployer_site: Option<&str>,
    body: Vec<u8>,
) -> Result<Request<Body>, Failure> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-ratatoskr-user-id", user.to_string())
        .header("x-correlation-id", "018f0000-0000-7000-8000-00000000c0c3");
    request = match deployer_site {
        Some(site) => request
            .header("x-ratatoskr-session-kind", "device")
            .header("x-ratatoskr-device-id", Uuid::now_v7().to_string())
            .header("x-ratatoskr-device-kind", "site_deployer")
            .header("x-ratatoskr-site-id", site),
        None => request.header("x-ratatoskr-session-kind", "browser"),
    };
    Ok(request.body(Body::from(body))?)
}

#[tokio::test]
async fn deploy_routes_require_an_available_verifier() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let deploys = format!("/v1/sites/{SITE}/deploys");

    let stored = call(
        &api,
        deploy_request(&deploys, fixture.owner, Some(SITE), bundle()?)?,
    )
    .await?;
    assert_eq!(stored.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(stored.code().as_deref(), Some("sites.verifier.unavailable"));

    let activated = call(
        &api,
        deploy_request(
            &format!("{deploys}/{COMMIT}/activate"),
            fixture.owner,
            None,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(activated.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        !fixture
            .deploys_root
            .path()
            .join(SITE)
            .join("current")
            .exists()
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn a_deployer_of_another_site_is_refused() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let refused = call(
        &api,
        deploy_request(
            &format!("/v1/sites/{SITE}/deploys"),
            fixture.owner,
            Some("site-other"),
            bundle()?,
        )?,
    )
    .await?;
    assert_eq!(refused.code().as_deref(), Some("sites.request.forbidden"));
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    let bad = call(
        &api,
        deploy_request(
            &format!("/v1/sites/{SITE}/deploys"),
            fixture.owner,
            Some(SITE),
            b"not JSON".to_vec(),
        )?,
    )
    .await?;
    assert_eq!(bad.code().as_deref(), Some("sites.request.invalid"));
    fixture.cleanup().await
}
