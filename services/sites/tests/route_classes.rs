//! SITE-025 task 5.30: the sites service refuses Edge's anonymous claim on every route outside
//! `content/` (workspace spec `site-route-classes`, ADR-0017 of `ratatoskr-platform`). SITE-027
//! task 9.1 adds the draft routes of the site owner session.

mod support;

use axum::http::{Method, StatusCode};
use support::http::{anonymous, call, owner};
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

#[tokio::test]
async fn anonymous_claim_is_refused_on_draft_routes() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let api = fixture.api().await?;

    for path in [
        format!("/v1/sites/{SITE}/drafts/sources/src_7Hq2kV9xR4mT1wZp"),
        format!("/v1/sites/{SITE}/drafts/pages/home"),
    ] {
        let answer = call(&api, anonymous(Method::GET, &path)?).await?;
        assert_eq!(
            answer.code().as_deref(),
            Some("sites.request.unauthenticated"),
            "{path}: the anonymous claim is refused"
        );
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED, "{path}");
        let body = String::from_utf8_lossy(&answer.body);
        assert!(
            !body.contains("markdown")
                && !body.contains("source_ref")
                && !body.contains("Start here"),
            "{path}: the refusal holds no source: {body}"
        );
    }
    fixture.cleanup().await
}

/// Edge reads the active manifest for an anonymous request with the claims of the owner who
/// approved the site, as the session kind `service` (Platform `site_capabilities::effective_set`).
#[tokio::test]
async fn internal_manifest_answers_the_service_claim_of_the_owner() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let mut request = owner(
        Method::GET,
        &format!("/internal/sites/{SITE}/manifest"),
        fixture.owner,
        Vec::new(),
    )?;
    request.headers_mut().insert(
        "x-ratatoskr-session-kind",
        axum::http::HeaderValue::from_static("service"),
    );
    let answer = call(&api, request).await?;
    assert_eq!(answer.status, StatusCode::OK, "the service claim reads it");
    assert_eq!(
        answer
            .json()?
            .get("site_id")
            .and_then(serde_json::Value::as_str),
        Some(SITE)
    );
    fixture.cleanup().await
}
