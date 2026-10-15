//! SITE-025 task 5.32: the four routes of the Content Delivery API, their cache headers and
//! conditional requests, and the one answer of every absence (workspace spec
//! `site-content-delivery`).

mod support;

use axum::http::{Method, StatusCode, header};
use ratatoskr_sites_contracts::{Page, Release};
use ratatoskr_sites_release::sha256_hex;
use support::bus::TestManifest;
use support::http::{Answer, anonymous, call, owner, submit};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

/// GET `path` with the anonymous claim of a router read now.
async fn get(fixture: &Fixture, path: &str) -> Result<Answer, Failure> {
    call(&fixture.api().await?, anonymous(Method::GET, path)?).await
}

/// The release document that the release route answers.
async fn release(fixture: &Fixture) -> Result<Release, Failure> {
    let answer = get(fixture, &format!("/v1/sites/{SITE}/content/release")).await?;
    assert_eq!(answer.status, StatusCode::OK, "the release route answers");
    Ok(serde_json::from_slice(&answer.body)?)
}

/// The `max-age` of the `Cache-Control` of `answer`, which must be `public`.
fn max_age(answer: &Answer) -> Result<u64, Failure> {
    let value = answer
        .headers
        .get(header::CACHE_CONTROL)
        .ok_or("the answer has Cache-Control")?
        .to_str()?;
    assert!(value.contains("public"), "public content: {value}");
    let age = value
        .split(',')
        .find_map(|part| part.trim().strip_prefix("max-age="))
        .ok_or("Cache-Control has max-age")?;
    Ok(age.parse()?)
}

#[tokio::test]
async fn the_four_routes_answer_for_an_approved_site() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let release = release(&fixture).await?;

    let page = release.pages.first().ok_or("the release has a page")?;
    let answer = get(
        &fixture,
        &format!("/v1/sites/{SITE}/content/pages/{}", page.slug.as_str()),
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK, "the page route answers");
    let parsed: Page = serde_json::from_slice(&answer.body)?;
    assert_eq!(parsed.slug, page.slug);

    let asset = release.assets.first().ok_or("the release has an asset")?;
    let answer = get(
        &fixture,
        &format!("/v1/sites/{SITE}/content/assets/{}", asset.sha256.as_str()),
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK, "the asset route answers");
    assert_eq!(sha256_hex(&answer.body), asset.sha256.as_str());
    assert_eq!(
        answer
            .headers
            .get(header::CONTENT_TYPE)
            .map(axum::http::HeaderValue::as_bytes),
        Some(asset.media_type.as_str().as_bytes()),
        "the asset has its media type"
    );

    let answer = get(
        &fixture,
        &format!("/v1/sites/{SITE}/content/search?q=other"),
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK, "the search route answers");
    let body = answer.json()?;
    assert!(body.get("results").is_some_and(serde_json::Value::is_array));
    assert!(
        body.get("has_more")
            .is_some_and(serde_json::Value::is_boolean)
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn page_of_earlier_release_is_not_found() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let path = format!("/v1/sites/{SITE}/content/pages/other");
    assert_eq!(get(&fixture, &path).await?.status, StatusCode::OK);

    // The note leaves the vault, so the next release has no page for it.
    fixture.remove("other.md")?;
    fixture.builder().build(&request(None)).await?;
    let answer = get(&fixture, &path).await?;
    assert_eq!(
        answer.status,
        StatusCode::NOT_FOUND,
        "the page left the release"
    );
    assert_eq!(answer.code().as_deref(), Some("sites.page_not_found"));
    fixture.cleanup().await
}

#[tokio::test]
async fn every_kind_of_absence_is_the_same_answer() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let missing = get(&fixture, "/v1/sites/never-was/content/release").await?;

    sqlx::query("update sites.approval_projection set capabilities = '{pages.read,assets.read}'")
        .execute(fixture.database.pool())
        .await?;
    let unrequested = get(
        &fixture,
        &format!("/v1/sites/{SITE}/content/search?q=other"),
    )
    .await?;
    sqlx::query("update sites.approval_projection set state = 'withdrawn'")
        .execute(fixture.database.pool())
        .await?;
    let withdrawn = get(&fixture, &format!("/v1/sites/{SITE}/content/release")).await?;
    sqlx::query("delete from sites.approval_projection")
        .execute(fixture.database.pool())
        .await?;
    let unapproved = get(&fixture, &format!("/v1/sites/{SITE}/content/release")).await?;

    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.code().as_deref(), Some("sites.site_not_found"));
    assert_eq!(
        missing
            .headers
            .get(header::CACHE_CONTROL)
            .map(axum::http::HeaderValue::as_bytes),
        Some(&b"no-store"[..]),
        "an absence is never stored"
    );
    for (kind, answer) in [
        ("an unrequested capability", &unrequested),
        ("a withdrawn site", &withdrawn),
        ("an unapproved site", &unapproved),
    ] {
        let mut headers = answer.headers.clone();
        let mut expected = missing.headers.clone();
        headers.remove(header::DATE);
        expected.remove(header::DATE);
        assert_eq!(answer.status, missing.status, "{kind}: the status");
        assert_eq!(headers, expected, "{kind}: the headers");
        assert_eq!(answer.body, missing.body, "{kind}: the body");
    }
    fixture.cleanup().await
}

#[tokio::test]
async fn unchanged_page_answers_304() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let path = format!("/v1/sites/{SITE}/content/pages/other");
    let first = get(&fixture, &path).await?;
    assert_eq!(first.status, StatusCode::OK);
    let tag = first
        .headers
        .get(header::ETAG)
        .ok_or("the page has an ETag")?
        .clone();

    let mut repeat = anonymous(Method::GET, &path)?;
    repeat
        .headers_mut()
        .insert(header::IF_NONE_MATCH, tag.clone());
    let second = call(&fixture.api().await?, repeat).await?;
    assert_eq!(
        second.status,
        StatusCode::NOT_MODIFIED,
        "no release came in between"
    );
    assert!(second.body.is_empty(), "a 304 has no body");
    assert_eq!(second.headers.get(header::ETAG), Some(&tag));
    fixture.cleanup().await
}

#[tokio::test]
async fn cache_lifetimes_are_bounded() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let release = release(&fixture).await?;
    let asset = release.assets.first().ok_or("the release has an asset")?;
    for path in [
        format!("/v1/sites/{SITE}/content/release"),
        format!("/v1/sites/{SITE}/content/pages/other"),
        format!("/v1/sites/{SITE}/content/search?q=other"),
    ] {
        let answer = get(&fixture, &path).await?;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert!(max_age(&answer)? <= 60, "{path}: at most 60 seconds");
    }
    let answer = get(
        &fixture,
        &format!("/v1/sites/{SITE}/content/assets/{}", asset.sha256.as_str()),
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(max_age(&answer)? <= 3600, "an asset: at most 3600 seconds");
    fixture.cleanup().await
}

/// The slugs of the search results of `q` on `api`.
async fn hits(api: &axum::Router, q: &str) -> Result<Vec<String>, Failure> {
    let answer = call(
        api,
        anonymous(
            Method::GET,
            &format!("/v1/sites/{SITE}/content/search?q={q}"),
        )?,
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK, "{q}: the search answers");
    Ok(answer
        .json()?
        .get("results")
        .and_then(serde_json::Value::as_array)
        .ok_or("the answer has results")?
        .iter()
        .filter_map(|result| result.get("slug")?.as_str().map(str::to_owned))
        .collect())
}

/// The slugs of the release document that `api` serves.
async fn listed(api: &axum::Router) -> Result<Vec<String>, Failure> {
    let answer = call(
        api,
        anonymous(Method::GET, &format!("/v1/sites/{SITE}/content/release"))?,
    )
    .await?;
    assert_eq!(answer.status, StatusCode::OK, "the release route answers");
    let release: Release = serde_json::from_slice(&answer.body)?;
    Ok(release
        .pages
        .iter()
        .map(|page| page.slug.as_str().to_owned())
        .collect())
}

#[tokio::test]
async fn removed_publication_stops_at_once() -> Outcome {
    let fixture = Fixture::new().await?;
    // A second publication over the same connection, which selects the private folder.
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         included_folders, created_at) values ($1, 'pub-s', $2, 'all', '{private}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await?;
    let capabilities = ["pages.read", "assets.read", "search.publication"];
    let both = TestManifest::of("r.example.test", &["pub-r", "pub-s"], &capabilities);
    fixture.approve_directly(&both).await?;
    fixture.builder().build(&request(None)).await?;
    let api = fixture.api().await?;
    let private = format!("/v1/sites/{SITE}/content/pages/private/zq7plannote");
    let page = call(&api, anonymous(Method::GET, &private)?).await?;
    assert_eq!(page.status, StatusCode::OK, "pub-s publishes the page");
    assert_eq!(hits(&api, "zq7planbody").await?, ["private/zq7plannote"]);
    let release = release(&fixture).await?;
    let asset = release.assets.first().ok_or("the release has an asset")?;

    // The owner removes pub-s from the manifest: a removal needs no approval.
    let narrower = TestManifest::of("r.example.test", &["pub-r"], &capabilities);
    assert_eq!(
        submit(&api, fixture.owner, &narrower).await?.status,
        StatusCode::OK
    );
    let page = call(&api, anonymous(Method::GET, &private)?).await?;
    assert_eq!(
        page.status,
        StatusCode::NOT_FOUND,
        "the page of pub-s stops at once"
    );
    assert_eq!(page.code().as_deref(), Some("sites.page_not_found"));
    assert_eq!(hits(&api, "zq7planbody").await?, Vec::<String>::new());
    assert!(
        !listed(&api)
            .await?
            .contains(&"private/zq7plannote".to_owned())
    );

    // The owner deletes pub-r.
    let removed = call(
        &api,
        owner(
            Method::DELETE,
            &format!("/v1/sites/{SITE}/publications/pub-r"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let page = call(
        &api,
        anonymous(
            Method::GET,
            &format!("/v1/sites/{SITE}/content/pages/other"),
        )?,
    )
    .await?;
    assert_eq!(
        page.status,
        StatusCode::NOT_FOUND,
        "the pages of pub-r stop at once"
    );
    let asset = call(
        &api,
        anonymous(
            Method::GET,
            &format!("/v1/sites/{SITE}/content/assets/{}", asset.sha256.as_str()),
        )?,
    )
    .await?;
    assert_eq!(
        asset.status,
        StatusCode::NOT_FOUND,
        "the assets of pub-r stop at once"
    );
    assert_eq!(asset.code().as_deref(), Some("sites.asset_not_found"));
    assert_eq!(hits(&api, "target").await?, Vec::<String>::new());
    assert_eq!(listed(&api).await?, Vec::<String>::new());
    assert_eq!(fixture.release_count().await?, 1, "no build ran");
    fixture.cleanup().await
}
