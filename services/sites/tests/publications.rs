//! The publication routes of the owner (local tasks 3.3 and 3.4 of `site-025-delivery`): the web
//! client writes, reads and removes the publications of a site.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::http::{call, owner};
use support::{Failure, Fixture, SITE};

type Outcome = Result<(), Failure>;

#[tokio::test]
async fn owner_writes_reads_and_removes_a_publication() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let path = format!("/v1/sites/{SITE}/publications/handbook");
    let connection = fixture.index.connection();
    let body = json!({
        "connection_id": connection,
        "mode": "flagged",
        "release_mode": "manual",
        "included_folders": ["Handbook"],
        "excluded_folders": ["Handbook/Drafts"],
        "included_tags": ["public"],
        "frontmatter_allow": ["summary"],
    });
    let written = call(
        &api,
        owner(
            Method::PUT,
            &path,
            fixture.owner,
            serde_json::to_vec(&body)?,
        )?,
    )
    .await?;
    assert_eq!(written.status, StatusCode::OK, "the publication is stored");
    let mut expected = body.clone();
    expected["site_id"] = json!(SITE);
    expected["publication_id"] = json!("handbook");
    expected["state"] = json!("active");
    assert_eq!(
        written.json()?,
        expected,
        "the answer is the stored publication"
    );

    let read = call(&api, owner(Method::GET, &path, fixture.owner, Vec::new())?).await?;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(
        read.json()?,
        expected,
        "a read gives the stored publication"
    );

    let list = call(
        &api,
        owner(
            Method::GET,
            &format!("/v1/sites/{SITE}/publications"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    let ids: Vec<String> = list
        .json()?
        .get("publications")
        .and_then(serde_json::Value::as_array)
        .ok_or("the list holds publications")?
        .iter()
        .filter_map(|publication| {
            publication
                .get("publication_id")?
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(ids, ["handbook", "pub-r"], "the list is in ascending order");

    let removed = call(
        &api,
        owner(Method::DELETE, &path, fixture.owner, Vec::new())?,
    )
    .await?;
    assert_eq!(
        removed.status,
        StatusCode::NO_CONTENT,
        "the publication is removed"
    );
    let gone = call(&api, owner(Method::GET, &path, fixture.owner, Vec::new())?).await?;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.code().as_deref(), Some("sites.publication_not_found"));
    fixture.cleanup().await
}

#[tokio::test]
async fn invalid_publication_is_refused() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let connection = fixture.index.connection();
    let valid = json!({
        "connection_id": connection,
        "mode": "all",
        "release_mode": "automatic",
        "included_folders": [],
        "excluded_folders": [],
        "included_tags": [],
        "frontmatter_allow": [],
    });
    let mut unknown_mode = valid.clone();
    unknown_mode["mode"] = json!("everything");
    let mut extra = valid.clone();
    extra["publish_all"] = json!(true);
    for (path, body) in [
        (
            format!("/v1/sites/{SITE}/publications/handbook"),
            unknown_mode,
        ),
        (format!("/v1/sites/{SITE}/publications/handbook"), extra),
        (
            format!("/v1/sites/{SITE}/publications/No_Such"),
            valid.clone(),
        ),
    ] {
        let answer = call(
            &api,
            owner(
                Method::PUT,
                &path,
                fixture.owner,
                serde_json::to_vec(&body)?,
            )?,
        )
        .await?;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{path} {body}");
        assert_eq!(answer.code().as_deref(), Some("sites.request.invalid"));
    }
    let unknown_site = call(
        &api,
        owner(
            Method::PUT,
            "/v1/sites/no-site/publications/handbook",
            fixture.owner,
            serde_json::to_vec(&valid)?,
        )?,
    )
    .await?;
    assert_eq!(unknown_site.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown_site.code().as_deref(), Some("sites.site_not_found"));
    fixture.cleanup().await
}
