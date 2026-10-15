//! SITE-027 task 9.3: a change of a note that a publication of a site selects becomes one
//! `evt.sites.draft.changed.v1` in the outbox, which Edge sends to the owner sessions of the site as
//! `draft.changed` (workspace design D5, spec `site-live-events`).

mod support;

use axum::http::Method;
use ratatoskr_identifiers::{BlobRef, DigestHex, MediaType, NotesConnectionId, NotesFileId};
use ratatoskr_notes_contracts::{FileChangeKind, FileChanged, VaultPath};
use ratatoskr_sites::consumer::Durable;
use support::bus::{TestBus, event};
use support::http::{call, site_owner};
use support::{Failure, Fixture, SITE, request};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// `evt.notes.file.changed.v1` for the file `path` of the fixture connection, as it is now.
fn change(fixture: &Fixture, path: &str, sequence: u64) -> Result<Vec<u8>, Failure> {
    let files = fixture.index.files_now()?;
    let file = files
        .iter()
        .find(|file| file.path == path)
        .ok_or("the fixture holds the file")?;
    let revision: &BlobRef = &file.revision;
    let payload = FileChanged {
        connection_id: NotesConnectionId(fixture.index.connection()),
        file_id: NotesFileId(Uuid::now_v7()),
        connection_sequence: sequence,
        change_kind: FileChangeKind::Updated,
        path: VaultPath::parse(path)?,
        previous_path: None,
        revision: DigestHex::parse(revision.digest.hex.as_str())?,
        media_type: MediaType::parse(revision.media_type.as_str())?,
        blob: Some(revision.clone()),
    };
    event("ratatoskr-notes", fixture.owner, &payload)
}

/// The payloads of the draft change events in the outbox, in order.
async fn draft_changes(fixture: &Fixture) -> Result<Vec<serde_json::Value>, Failure> {
    Ok(sqlx::query_scalar(
        "select payload from sites.outbox where subject = 'evt.sites.draft.changed.v1' \
         order by position",
    )
    .fetch_all(fixture.database.pool())
    .await?)
}

#[tokio::test]
async fn file_change_in_publication_emits_draft_changed() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    let api = fixture.api_of(&running.shared).await?;

    fixture.change(
        "home.md",
        "---\ntitle: Home\npublish: true\n---\n# Home\n\nThe owner changed this.\n",
    )?;
    bus.publish("evt.notes.file.changed.v1", change(&fixture, "home.md", 1)?)
        .await?;
    bus.settled(Durable::NoteChanges).await?;

    let rows = draft_changes(&fixture).await?;
    assert_eq!(
        rows.len(),
        1,
        "one draft change for the selected note: {rows:?}"
    );
    let scope = call(
        &api,
        site_owner(
            Method::GET,
            &format!(
                "/internal/sites/{SITE}/scope?connection_id={}&path=home.md",
                fixture.index.connection()
            ),
            fixture.owner,
        )?,
    )
    .await?
    .json()?;
    let revision = fixture
        .index
        .files_now()?
        .into_iter()
        .find(|file| file.path == "home.md")
        .ok_or("the fixture holds home.md")?
        .revision
        .digest
        .hex;
    let envelope = rows.first().ok_or("one row")?;
    let payload = envelope
        .get("payload")
        .ok_or("the envelope has a payload")?;
    assert_eq!(payload.get("site_id"), Some(&serde_json::json!(SITE)));
    assert_eq!(payload.get("source_ref"), scope.get("source_ref"));
    assert_eq!(
        payload.get("revision").and_then(serde_json::Value::as_str),
        Some(revision.as_str())
    );
    assert_eq!(payload.get("slug"), Some(&serde_json::json!("home")));
    assert_eq!(
        envelope
            .get("event_type")
            .and_then(serde_json::Value::as_str),
        Some("sites.draft.changed.v1")
    );
    let text = envelope.to_string();
    assert!(
        !text.contains("home.md") && !text.contains("changed this"),
        "no path and no content: {text}"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn file_change_outside_publications_emits_nothing() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;

    fixture.change("private/zq7plannote.md", "# Plan\n\nStill private.\n")?;
    bus.publish(
        "evt.notes.file.changed.v1",
        change(&fixture, "private/zq7plannote.md", 1)?,
    )
    .await?;
    bus.settled(Durable::NoteChanges).await?;

    assert_eq!(
        draft_changes(&fixture).await?,
        Vec::<serde_json::Value>::new(),
        "no draft change for a note that no publication selects"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
