//! SITE-027 task 9.1: the loopback scope query of the sites service (workspace change
//! `add-site-owner-editing-and-live`, design D6). Edge resolves an opaque `source_ref` to the
//! connection and the path of a note that a publication of the site selects, and a note to its
//! `source_ref`; a note outside the publications of the site is out of scope.

mod support;

use std::net::SocketAddr;

use axum::Router;
use axum::http::{Method, StatusCode};
use support::http::{call, call_from, site_owner};
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// The `source_ref` that the scope query gives for `path` of the fixture connection, or `None`
/// when it answers 404.
async fn source_ref_of(
    api: &Router,
    fixture: &Fixture,
    path: &str,
) -> Result<Option<String>, Failure> {
    let query = format!(
        "/internal/sites/{SITE}/scope?connection_id={}&path={}",
        fixture.index.connection(),
        path.replace('/', "%2F").replace(' ', "%20")
    );
    let answer = call(api, site_owner(Method::GET, &query, fixture.owner)?).await?;
    if answer.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    assert_eq!(answer.status, StatusCode::OK, "the scope query of {path}");
    Ok(answer
        .json()?
        .get("source_ref")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned))
}

/// The connection and the path that the source query gives for `source_ref`, or `None` when it
/// answers 404.
async fn source_of(
    api: &Router,
    fixture: &Fixture,
    source_ref: &str,
) -> Result<Option<(Uuid, String)>, Failure> {
    let path = format!("/internal/sites/{SITE}/sources/{source_ref}");
    let answer = call(api, site_owner(Method::GET, &path, fixture.owner)?).await?;
    if answer.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the source query of {source_ref}"
    );
    let body = answer.json()?;
    let connection = body
        .get("connection_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("the answer names the connection")?;
    let note = body
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or("the answer names the path")?;
    Ok(Some((Uuid::parse_str(connection)?, note.to_owned())))
}

/// The scope query of `path`, then the source query of its `source_ref`.
async fn resolved(
    api: &Router,
    fixture: &Fixture,
    path: &str,
) -> Result<Option<(Uuid, String)>, Failure> {
    match source_ref_of(api, fixture, path).await? {
        Some(source_ref) => source_of(api, fixture, &source_ref).await,
        None => Ok(None),
    }
}

#[tokio::test]
async fn note_in_publication_resolves_to_connection_and_path() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;

    assert_eq!(
        resolved(&api, &fixture, "home.md").await?,
        Some((fixture.index.connection(), "home.md".to_owned())),
        "a note that pub-r selects resolves to its connection and path"
    );
    let first = source_ref_of(&api, &fixture, "home.md").await?;
    let second = source_ref_of(&api, &fixture, "home.md").await?;
    assert_eq!(first, second, "a note keeps one source_ref");
    assert_eq!(
        source_of(&api, &fixture, "src_unknown").await?,
        None,
        "an unknown source_ref is not found"
    );

    let foreign = call_from(
        &api,
        site_owner(
            Method::GET,
            &format!("/internal/sites/{SITE}/sources/src_unknown"),
            fixture.owner,
        )?,
        SocketAddr::from(([192, 0, 2, 7], 40_000)),
    )
    .await?;
    assert_eq!(
        foreign.status,
        StatusCode::FORBIDDEN,
        "the scope query answers a loopback peer only"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn owned_note_outside_publications_is_out_of_scope() -> Outcome {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;

    assert_eq!(
        resolved(&api, &fixture, "home.md").await?,
        Some((fixture.index.connection(), "home.md".to_owned())),
        "the control: a note that pub-r selects is in scope"
    );
    assert_eq!(
        resolved(&api, &fixture, "private/zq7plannote.md").await?,
        None,
        "a note of the same connection that no publication selects is out of scope"
    );
    fixture.cleanup().await
}
