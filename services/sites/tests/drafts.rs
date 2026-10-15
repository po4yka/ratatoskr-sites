//! SITE-027 task 9.2, the answer of the draft routes (workspace spec `site-owner-editing`, "The
//! owner reads the source of a page in scope"): the Markdown source, the `source_ref`, the
//! revision that the notes index reports and the slug, to a site owner session only.

mod support;

use axum::http::{Method, StatusCode};
use ratatoskr_sites_release::sha256_hex;
use support::http::{call, owner, site_owner};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

const HOME: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/privacy-vault/vault/home.md"
);

#[tokio::test]
async fn owner_reads_the_source_of_a_page_and_of_a_note() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    // A note that pub-r selects and that no release holds yet.
    fixture.add_note("fresh.md", "# Fresh\n\nNot released yet.\n")?;
    let api = fixture.api().await?;

    let page = call(
        &api,
        site_owner(
            Method::GET,
            &format!("/v1/sites/{SITE}/drafts/pages/home"),
            fixture.owner,
        )?,
    )
    .await?;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        page.headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let draft = page.json()?;
    let markdown = std::fs::read(HOME)?;
    assert_eq!(
        draft.get("markdown").and_then(serde_json::Value::as_str),
        Some(String::from_utf8(markdown.clone())?.as_str())
    );
    assert_eq!(
        draft.get("revision").and_then(serde_json::Value::as_str),
        Some(sha256_hex(&markdown).as_str())
    );
    assert_eq!(
        draft.get("slug").and_then(serde_json::Value::as_str),
        Some("home")
    );
    let source_ref = draft
        .get("source_ref")
        .and_then(serde_json::Value::as_str)
        .ok_or("the draft names its source_ref")?;
    assert!(
        source_ref.starts_with("src_") && !source_ref.contains("home"),
        "an opaque reference: {source_ref}"
    );

    let by_ref = call(
        &api,
        site_owner(
            Method::GET,
            &format!("/v1/sites/{SITE}/drafts/sources/{source_ref}"),
            fixture.owner,
        )?,
    )
    .await?;
    assert_eq!(by_ref.status, StatusCode::OK);
    assert_eq!(by_ref.json()?, draft, "both routes answer one draft");

    let fresh_ref = call(
        &api,
        site_owner(
            Method::GET,
            &format!(
                "/internal/sites/{SITE}/scope?connection_id={}&path=fresh.md",
                fixture.index.connection()
            ),
            fixture.owner,
        )?,
    )
    .await?
    .json()?;
    let fresh_ref = fresh_ref
        .get("source_ref")
        .and_then(serde_json::Value::as_str)
        .ok_or("the scope query names the source_ref")?;
    let fresh = call(
        &api,
        site_owner(
            Method::GET,
            &format!("/v1/sites/{SITE}/drafts/sources/{fresh_ref}"),
            fixture.owner,
        )?,
    )
    .await?;
    assert_eq!(fresh.status, StatusCode::OK, "a note not yet released");
    let fresh = fresh.json()?;
    assert_eq!(fresh.get("slug"), None, "it has no slug");
    assert_eq!(
        fresh.get("markdown").and_then(serde_json::Value::as_str),
        Some("# Fresh\n\nNot released yet.\n")
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn draft_routes_answer_only_the_site_owner_session_of_the_owner() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let api = fixture.api().await?;
    let home = format!("/v1/sites/{SITE}/drafts/pages/home");

    let browser = call(&api, owner(Method::GET, &home, fixture.owner, Vec::new())?).await?;
    assert_eq!(
        browser.status,
        StatusCode::UNAUTHORIZED,
        "a primary session"
    );
    assert_eq!(
        browser.code().as_deref(),
        Some("sites.request.unauthenticated")
    );

    let stranger = call(&api, site_owner(Method::GET, &home, uuid::Uuid::now_v7())?).await?;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND, "another user");
    assert_eq!(stranger.code().as_deref(), Some("sites.site_not_found"));

    let private = call(
        &api,
        site_owner(
            Method::GET,
            &format!("/v1/sites/{SITE}/drafts/pages/private/zq7plannote"),
            fixture.owner,
        )?,
    )
    .await?;
    assert_eq!(private.status, StatusCode::NOT_FOUND, "an unselected note");
    assert_eq!(private.code().as_deref(), Some("sites.page_not_found"));
    assert!(!String::from_utf8_lossy(&private.body).contains("zq7"));
    fixture.cleanup().await
}
