//! The fixture of the release build tests: a disposable database with one site and one
//! publication, a notes blob root with the bytes of `fixtures/privacy-vault/`, a stub of the notes
//! internal index route, and an empty sites blob root.

#![allow(dead_code, reason = "each test file uses a part of the fixture")]

pub(crate) mod bus;
pub(crate) mod http;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ratatoskr_sites::notes_blobs::NotesBlobReader;
use ratatoskr_sites::notes_index::{IndexUnavailable, NotesIndex};
use ratatoskr_sites::release_build::{BuildRequest, ReleaseBuilder};
use ratatoskr_sites_persistence::test_support::TestDatabase;
use ratatoskr_sites_publications::IndexedFile;
use ratatoskr_sites_publications::test_support::{load_index, revision_of};
use uuid::Uuid;

pub(crate) type Failure = Box<dyn std::error::Error>;

pub(crate) const SITE: &str = "site-r";

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/privacy-vault");

/// The bound of a note read in a build: the default largest file of `ratatoskr-notes`, written
/// here apart from the service.
pub(crate) const MAX_NOTE_BYTES: u64 = 64 * 1024 * 1024;

/// A stub of the notes internal index route: one connection and its files.
#[derive(Debug, Clone)]
pub(crate) struct StubIndex {
    connection: Uuid,
    files: Arc<Mutex<Vec<IndexedFile>>>,
}

impl StubIndex {
    /// The connection of the stub.
    pub(crate) const fn connection(&self) -> Uuid {
        self.connection
    }

    /// The files of the stub as they are now.
    pub(crate) fn files_now(&self) -> Result<Vec<IndexedFile>, Failure> {
        Ok(self
            .files
            .lock()
            .map_err(|_| "the stub index is poisoned")?
            .clone())
    }
}

impl NotesIndex for StubIndex {
    async fn files(&self, connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexUnavailable> {
        if connection_id != self.connection {
            return Err(IndexUnavailable);
        }
        Ok(self.files.lock().map_err(|_| IndexUnavailable)?.clone())
    }
}

pub(crate) struct Fixture {
    pub(crate) database: TestDatabase,
    pub(crate) notes_root: tempfile::TempDir,
    pub(crate) sites_root: tempfile::TempDir,
    pub(crate) index: StubIndex,
    pub(crate) owner: Uuid,
}

/// The path of the blob `hex` under `root`.
pub(crate) fn blob_path(root: &Path, hex: &str) -> Result<PathBuf, Failure> {
    let (prefix, rest) = hex
        .split_at_checked(2)
        .ok_or("a digest has 64 hex digits")?;
    Ok(root.join("sha256").join(prefix).join(rest))
}

fn store(root: &Path, bytes: &[u8], hex: &str) -> Result<(), Failure> {
    let path = blob_path(root, hex)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

impl Fixture {
    /// A site `site-r` with one publication of the fixture vault in the mode `all`, which excludes
    /// `private/`.
    pub(crate) async fn new() -> Result<Self, Failure> {
        let database = TestDatabase::create().await?;
        let notes_root = tempfile::tempdir()?;
        let sites_root = tempfile::tempdir()?;
        let files = load_index(Path::new(FIXTURE))?;
        for file in &files {
            let bytes = std::fs::read(Path::new(FIXTURE).join("vault").join(&file.path))?;
            store(notes_root.path(), &bytes, file.revision.digest.hex.as_str())?;
        }
        let owner = Uuid::now_v7();
        let connection = Uuid::now_v7();
        sqlx::query(
            "insert into sites.sites (site_id, owner_id, created_at) values ($1, $2, now())",
        )
        .bind(SITE)
        .bind(owner)
        .execute(database.pool())
        .await?;
        sqlx::query(
            "insert into sites.publications (site_id, publication_id, connection_id, mode, \
             excluded_folders, created_at) values ($1, 'pub-r', $2, 'all', '{private}', now())",
        )
        .bind(SITE)
        .bind(connection)
        .execute(database.pool())
        .await?;
        Ok(Self {
            database,
            notes_root,
            sites_root,
            index: StubIndex {
                connection,
                files: Arc::new(Mutex::new(files)),
            },
            owner,
        })
    }

    pub(crate) fn builder(&self) -> ReleaseBuilder<StubIndex> {
        ReleaseBuilder::new(
            self.database.database.clone(),
            self.index.clone(),
            NotesBlobReader::new(self.notes_root.path()),
            self.sites_root.path().to_path_buf(),
        )
    }

    /// Give the note `path` the text `text`, as a new revision in notes.
    pub(crate) fn change(&self, path: &str, text: &str) -> Result<(), Failure> {
        let mut files = self
            .index
            .files
            .lock()
            .map_err(|_| "the stub index is poisoned")?;
        let file = files
            .iter_mut()
            .find(|file| file.path == path)
            .ok_or("the fixture holds the file")?;
        file.revision = revision_of(text.as_bytes(), "text/markdown")?;
        store(
            self.notes_root.path(),
            text.as_bytes(),
            file.revision.digest.hex.as_str(),
        )
    }

    /// Give the file `path` the bytes `bytes` with the media type `media_type`, as a new revision
    /// in notes.
    pub(crate) fn replace(
        &self,
        path: &str,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<(), Failure> {
        let mut files = self
            .index
            .files
            .lock()
            .map_err(|_| "the stub index is poisoned")?;
        let file = files
            .iter_mut()
            .find(|file| file.path == path)
            .ok_or("the fixture holds the file")?;
        file.revision = revision_of(bytes, media_type)?;
        store(
            self.notes_root.path(),
            bytes,
            file.revision.digest.hex.as_str(),
        )
    }

    /// Add the note `path` with the text `text` to the connection.
    pub(crate) fn add_note(&self, path: &str, text: &str) -> Result<(), Failure> {
        let revision = revision_of(text.as_bytes(), "text/markdown")?;
        store(
            self.notes_root.path(),
            text.as_bytes(),
            revision.digest.hex.as_str(),
        )?;
        self.index
            .files
            .lock()
            .map_err(|_| "the stub index is poisoned")?
            .push(IndexedFile {
                path: path.to_owned(),
                revision,
                frontmatter: None,
                tags: Vec::new(),
                headings: Vec::new(),
                blocks: Vec::new(),
                links: Vec::new(),
                embeds: Vec::new(),
            });
        Ok(())
    }

    /// The payloads of the operation reports in the outbox.
    pub(crate) async fn reports(&self) -> Result<Vec<serde_json::Value>, Failure> {
        Ok(sqlx::query_scalar(
            "select payload -> 'payload' from sites.outbox \
             where subject = 'evt.platform.operation.reported.v1' order by position",
        )
        .fetch_all(self.database.pool())
        .await?)
    }

    pub(crate) async fn release_count(&self) -> Result<i64, Failure> {
        Ok(
            sqlx::query_scalar("select count(*) from sites.releases where site_id = $1")
                .bind(SITE)
                .fetch_one(self.database.pool())
                .await?,
        )
    }

    pub(crate) async fn document(&self, release_id: &str) -> Result<Vec<u8>, Failure> {
        Ok(
            sqlx::query_scalar("select document from sites.releases where release_id = $1")
                .bind(release_id)
                .fetch_one(self.database.pool())
                .await?,
        )
    }

    pub(crate) async fn cleanup(self) -> Result<(), Failure> {
        Ok(self.database.cleanup().await?)
    }
}

pub(crate) fn request(operation_id: Option<Uuid>) -> BuildRequest {
    BuildRequest {
        site_id: SITE.to_owned(),
        operation_id,
    }
}
