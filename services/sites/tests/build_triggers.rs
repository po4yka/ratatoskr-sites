//! SITE-025 task 5.22: a build starts only from a command or from a change of a selected file of an
//! automatic publication that already has a release, and changes during a build cause one more
//! build (workspace spec `site-publications`, design D6). Against a real `JetStream`.

mod support;

use std::time::Duration;

use ratatoskr_identifiers::{BlobRef, DigestHex, MediaType, NotesConnectionId, NotesFileId};
use ratatoskr_notes_contracts::{FileChangeKind, FileChanged, VaultPath};
use ratatoskr_sites::consumer::Durable;
use support::bus::{TestBus, build_command, event};
use support::{Failure, Fixture, SITE};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// How long a test waits for the build worker to finish what the durables delivered.
const PATIENCE: Duration = Duration::from_secs(30);

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

/// Publish a build command and wait until the worker has no request left.
async fn command(fixture: &Fixture, bus: &TestBus) -> Outcome {
    bus.publish(
        "cmd.sites.release.build_requested.v1",
        build_command(SITE, fixture.owner, Uuid::now_v7())?,
    )
    .await?;
    bus.settled(Durable::ReleaseBuilds).await?;
    idle(fixture).await
}

/// Wait until the build worker has no request row left.
async fn idle(fixture: &Fixture) -> Outcome {
    tokio::time::timeout(PATIENCE, async {
        loop {
            let left: i64 = sqlx::query_scalar("select count(*) from sites.build_requests")
                .fetch_one(fixture.database.pool())
                .await?;
            if left == 0 {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "the build worker did not finish its requests")?
}

/// Publish a change of each of `paths` and wait until the durable acknowledged all of them.
async fn changes(fixture: &Fixture, bus: &TestBus, paths: &[&str]) -> Outcome {
    for (sequence, path) in (1..).zip(paths) {
        bus.publish(
            "evt.notes.file.changed.v1",
            change(fixture, path, sequence)?,
        )
        .await?;
    }
    bus.settled(Durable::NoteChanges).await
}

#[tokio::test]
async fn unrelated_change_starts_no_build() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    command(&fixture, &bus).await?;
    assert_eq!(running.builds(), 1, "the command builds the first release");

    // `private/` is excluded, so the publication neither selects the note now nor selected it in
    // the current release.
    changes(&fixture, &bus, &["private/zq7plannote.md"]).await?;
    idle(&fixture).await?;
    assert_eq!(
        running.builds(),
        1,
        "a change of an unselected file starts no build"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn changes_during_a_build_coalesce_into_one() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, 1).await?;
    command(&fixture, &bus).await?;
    assert_eq!(running.builds(), 1, "the command builds the first release");

    // A change starts a build, which waits at the gate while five more changes arrive.
    changes(&fixture, &bus, &["a.md"]).await?;
    running.started(2).await?;
    changes(&fixture, &bus, &["a.md", "b.md", "c.md", "d.md", "home.md"]).await?;
    running.open(64);
    idle(&fixture).await?;
    assert_eq!(
        running.builds(),
        3,
        "the five changes during the second build make exactly one more build"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn manual_publication_ignores_file_changes() -> Outcome {
    let fixture = Fixture::new().await?;
    sqlx::query("update sites.publications set release_mode = 'manual'")
        .execute(fixture.database.pool())
        .await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;
    command(&fixture, &bus).await?;
    assert_eq!(running.builds(), 1, "the command builds the first release");

    changes(&fixture, &bus, &["a.md"]).await?;
    idle(&fixture).await?;
    assert_eq!(
        running.builds(),
        1,
        "a change of a selected file of a manual publication starts no build"
    );

    command(&fixture, &bus).await?;
    assert_eq!(running.builds(), 2, "the next command builds");

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}

#[tokio::test]
async fn first_release_needs_an_explicit_request() -> Outcome {
    let fixture = Fixture::new().await?;
    let bus = TestBus::new().await?;
    let running = fixture.start(&bus, usize::MAX >> 4).await?;

    // The automatic publication has no release, and `a.md` carries `publish: true`.
    changes(&fixture, &bus, &["a.md"]).await?;
    idle(&fixture).await?;
    assert_eq!(
        running.builds(),
        0,
        "a file change does not build a publication that has no release"
    );
    assert_eq!(fixture.release_count().await?, 0, "no release exists");

    command(&fixture, &bus).await?;
    assert_eq!(running.builds(), 1, "the command builds the first release");
    assert_eq!(
        fixture.release_count().await?,
        1,
        "the first release exists"
    );

    running.stop().await;
    bus.cleanup().await?;
    fixture.cleanup().await
}
