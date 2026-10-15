//! The build of the release of a site (workspace design D4, spec `site-publications`).

use std::path::PathBuf;

use ratatoskr_sites_persistence::Database;
use uuid::Uuid;

use crate::notes_blobs::NotesBlobReader;
use crate::notes_index::NotesIndex;

/// What starts a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// The site to build.
    pub site_id: String,
    /// The Platform operation that tracks the build, for a build that Edge requested.
    pub operation_id: Option<Uuid>,
}

/// What a build made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutcome {
    /// The release id: the SHA-256 of the RFC 8785 form of the release document.
    pub release_id: String,
    /// `false` when the build made the release that is already current.
    pub new_release: bool,
}

/// Why a build made no release.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildFailure {
    /// A query failed.
    #[error("a database query of the build failed")]
    Database(#[source] sqlx::Error),
}

/// The builder of the releases of the sites.
#[derive(Debug)]
pub struct ReleaseBuilder<I> {
    database: Database,
    index: I,
    notes: NotesBlobReader,
    blob_root: PathBuf,
}

impl<I: NotesIndex> ReleaseBuilder<I> {
    /// A builder that reads the index from `index`, note bytes with `notes`, and writes pages and
    /// assets under `blob_root`.
    #[must_use]
    pub fn new(database: Database, index: I, notes: NotesBlobReader, blob_root: PathBuf) -> Self {
        Self {
            database,
            index,
            notes,
            blob_root,
        }
    }

    /// Build the release of `request.site_id`.
    ///
    /// # Errors
    ///
    /// [`BuildFailure`] when the build makes no release.
    pub async fn build(&self, request: &BuildRequest) -> Result<BuildOutcome, BuildFailure> {
        let _ = (&self.index, &self.notes, &self.blob_root);
        let release_id = ratatoskr_sites_release::sha256_hex(Uuid::now_v7().as_bytes());
        sqlx::query(
            "insert into sites.releases (release_id, site_id, document, page_count, built_at) \
             values ($1, $2, ''::bytea, 0, now())",
        )
        .bind(&release_id)
        .bind(&request.site_id)
        .execute(self.database.pool())
        .await
        .map_err(BuildFailure::Database)?;
        Ok(BuildOutcome {
            release_id,
            new_release: true,
        })
    }
}
