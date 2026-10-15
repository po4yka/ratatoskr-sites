//! SITE-025 task 5.30: the sites service refuses Edge's anonymous claim on every route outside
//! `content/` (workspace spec `site-route-classes`, ADR-0017 of `ratatoskr-platform`).

mod support;

use axum::http::{Method, StatusCode};
use support::http::{anonymous, call};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

#[tokio::test]
async fn anonymous_claim_is_refused_outside_content() -> Outcome {
    let fixture = Fixture::new().await?;
    let built = fixture.builder().build(&request(None)).await?;
    let api = fixture.api().await?;

    let answer = call(
        &api,
        anonymous(Method::GET, &format!("/v1/sites/{SITE}/releases"))?,
    )
    .await?;
    assert_eq!(
        answer.status,
        StatusCode::UNAUTHORIZED,
        "the anonymous claim is refused"
    );
    assert_eq!(
        answer.code().as_deref(),
        Some("sites.request.unauthenticated")
    );
    let body = String::from_utf8_lossy(&answer.body);
    assert!(
        !body.contains(&built.release_id) && !body.contains("releases"),
        "the refusal holds no release data: {body}"
    );
    fixture.cleanup().await
}
