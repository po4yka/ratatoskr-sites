//! SITE-025 task 5.48: the sites service reads the index of a vault connection from the notes
//! internal index route `GET /internal/connections/{connection}/files`, with a timeout and a bound
//! on the answer. Against a stub HTTP server that answers as `ratatoskr-notes` does.

use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ratatoskr_sites::notes_index::{HttpNotesIndex, IndexError, NotesIndex};
use ratatoskr_sites_publications::LinkForm;
use serde_json::json;
use uuid::Uuid;

type Failure = Box<dyn std::error::Error>;
type Outcome = Result<(), Failure>;

const KNOWN: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0b01);
const SLOW: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0b02);
const LARGE: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0b03);
const ROUTE_GONE: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0b04);
const MOVED: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0b05);

/// The largest answer that the index under test reads.
const MAX_BYTES: usize = 64 * 1024;

async fn files(Path(connection): Path<Uuid>) -> Response {
    if connection == SLOW {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    if connection == ROUTE_GONE {
        // The answer of notes to a peer that is not loopback, or of an older notes without the
        // route: an unknown route, with an empty body.
        return StatusCode::NOT_FOUND.into_response();
    }
    if connection == MOVED {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [(axum::http::header::LOCATION, "http://192.0.2.10/files")],
        )
            .into_response();
    }
    if connection == LARGE {
        return Json(json!({ "files": [], "padding": "x".repeat(MAX_BYTES * 2) })).into_response();
    }
    if connection != KNOWN && connection != SLOW {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "code": "notes.connection.not_found",
                "message": "The connection does not exist.",
                "retryable": false,
            })),
        )
            .into_response();
    }
    Json(json!({ "files": [{
        "path": "blog/hello.md",
        "revision": {
            "owner_service": "ratatoskr-notes",
            "digest": { "algorithm": "sha256", "hex": "1f2e3d4c5b6a70811f2e3d4c5b6a70811f2e3d4c5b6a70811f2e3d4c5b6a7081" },
            "media_type": "text/markdown",
            "length_bytes": 2048,
        },
        "frontmatter": { "publish": true },
        "tags": ["#public"],
        "headings": [{ "level": 1, "heading": "Hello" }],
        "blocks": ["intro"],
        "links": [{ "link": "Other", "display_text": "Other", "form": "wiki", "target": "blog/other.md" }],
        "embeds": [{ "link": "cover.png", "display_text": "cover.png", "form": "wiki", "target": null }],
    }]}))
    .into_response()
}

/// A stub notes domain listener, and its base URL.
async fn stub() -> Result<String, Failure> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new().route("/internal/connections/{connection}/files", get(files));
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(format!("http://{address}"))
}

fn index(base: &str) -> HttpNotesIndex {
    HttpNotesIndex::new(base, Duration::from_millis(500), MAX_BYTES)
}

#[tokio::test]
async fn files_of_a_connection_are_read() -> Outcome {
    let base = stub().await?;
    let files = index(&base).files(KNOWN).await;
    let files = files.map_err(|error| format!("the index is read: {error}"));
    assert!(files.is_ok(), "{files:?}");
    let files = files?;
    assert_eq!(files.len(), 1);
    let file = files.first().ok_or("one file")?;
    assert_eq!(file.path, "blog/hello.md");
    assert_eq!(file.frontmatter_value("publish"), Some(&json!(true)));
    assert_eq!(file.tags, ["#public"]);
    let link = file.links.first().ok_or("one link")?;
    assert_eq!(link.form, LinkForm::Wiki);
    assert_eq!(link.target.as_deref(), Some("blog/other.md"));
    assert_eq!(
        file.embeds.first().and_then(|embed| embed.target.clone()),
        None
    );
    Ok(())
}

#[tokio::test]
async fn unknown_connection_is_not_found() -> Outcome {
    let base = stub().await?;
    assert_eq!(
        index(&base).files(Uuid::now_v7()).await,
        Err(IndexError::ConnectionNotFound),
        "a 404 of notes is a connection that does not exist"
    );
    Ok(())
}

#[tokio::test]
async fn slow_index_is_unavailable() -> Outcome {
    let base = stub().await?;
    let started = tokio::time::Instant::now();
    assert_eq!(index(&base).files(SLOW).await, Err(IndexError::Unavailable));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the read stops at its timeout"
    );
    Ok(())
}

#[tokio::test]
async fn oversized_index_is_unavailable() -> Outcome {
    let base = stub().await?;
    assert_eq!(
        index(&base).files(LARGE).await,
        Err(IndexError::Unavailable),
        "an answer over the bound is not read"
    );
    Ok(())
}

#[tokio::test]
async fn other_not_found_is_unavailable() -> Outcome {
    let base = stub().await?;
    assert_eq!(
        index(&base).files(ROUTE_GONE).await,
        Err(IndexError::Unavailable),
        "a 404 without the code notes.connection.not_found is not a missing connection"
    );
    Ok(())
}

#[tokio::test]
async fn redirect_is_unavailable() -> Outcome {
    let base = stub().await?;
    assert_eq!(
        index(&base).files(MOVED).await,
        Err(IndexError::Unavailable),
        "a redirect is not followed"
    );
    Ok(())
}
