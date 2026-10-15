//! The build of the release of a site (workspace design D4, spec `site-publications`).
//!
//! A build reads the index of each vault connection of the site's publications, reads the bytes of
//! each selected note with a bound, and gives both to the release builder of
//! `ratatoskr-sites-release`, which parses, filters and renders every selected page. The release
//! document is serialised in its RFC 8785 form, and the SHA-256 of those bytes is the release id.
//! A build whose release id is the current one stores nothing and names the current release.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ratatoskr_sites_contracts::{PublicationId, SiteId};
use ratatoskr_sites_persistence::Database;
use ratatoskr_sites_publications::{BuildError, IndexedFile, Publication, PublicationMode, select};
use ratatoskr_sites_release::{Built, PublicationInput, SiteInput, build_release, selected_notes};
use uuid::Uuid;

use crate::events::{self, BuildReport};
use crate::notes_blobs::{NotesBlobError, NotesBlobReader};
use crate::notes_index::NotesIndex;
use crate::sites_blobs::SitesBlobs;

/// The largest note or attachment that a build reads: 64 MiB, the default largest file that
/// `ratatoskr-notes` indexes (`RATATOSKR__FILES__MAX_FILE_BYTES`). A larger reference fails the
/// build with `sites.blob_length` before anything is read.
pub const MAX_NOTE_BYTES: u64 = 64 * 1024 * 1024;

/// What starts a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// The site to build.
    pub site_id: String,
    /// The Platform operation that tracks the build, for a build that Edge requested.
    pub operation_id: Option<Uuid>,
}

/// One build of a site, for every request that the build worker took together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildJob {
    /// The site to build.
    pub site_id: String,
    /// The Platform operations of the build commands among the requests; each gets the outcome.
    pub operation_ids: Vec<Uuid>,
    /// `true` when a build command is among the requests, `false` when only file changes are.
    pub explicit: bool,
}

impl From<&BuildRequest> for BuildJob {
    fn from(request: &BuildRequest) -> Self {
        Self {
            site_id: request.site_id.clone(),
            operation_ids: request.operation_id.into_iter().collect(),
            explicit: true,
        }
    }
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
    /// The site does not exist.
    #[error("the site does not exist")]
    SiteNotFound,
    /// A publication that the build reads is suspended: its vault connection was removed.
    #[error("a publication of the site is suspended")]
    PublicationSuspended,
    /// The build reads no publication.
    #[error("the build reads no publication of the site")]
    PublicationNotFound,
    /// A stored publication does not satisfy the contract.
    #[error("a publication of the site is not valid")]
    PublicationInvalid,
    /// The notes index of a connection could not be read.
    #[error("the notes index is unavailable")]
    IndexUnavailable,
    /// The bytes of a note could not be read or verified.
    #[error("a note could not be read")]
    Note(#[source] NotesBlobError),
    /// The release builder made no release.
    #[error("the release builder made no release")]
    Build(#[source] BuildError),
    /// A page or an asset could not be stored.
    #[error("the release could not be stored")]
    Store(#[source] std::io::Error),
    /// A query failed.
    #[error("a database query of the build failed")]
    Database(#[source] sqlx::Error),
}

impl BuildFailure {
    /// The stable class of the failure, for the operation report.
    #[must_use]
    pub fn class(&self) -> String {
        match self {
            Self::SiteNotFound => "sites.site_not_found".to_owned(),
            Self::PublicationNotFound => "sites.publication_not_found".to_owned(),
            Self::PublicationSuspended => "sites.publication_suspended".to_owned(),
            Self::PublicationInvalid => "sites.publication_invalid".to_owned(),
            Self::IndexUnavailable => "sites.notes_index_unavailable".to_owned(),
            Self::Note(error) => format!("sites.{}", error.class()),
            Self::Build(error) => error.class.to_owned(),
            Self::Store(_) | Self::Database(_) => "sites.service_unavailable".to_owned(),
        }
    }

    /// What the owner reads in the operation. The detail of a build failure names only files that
    /// the publication selects.
    fn message(&self) -> String {
        match self {
            Self::Build(error) => error.detail.clone(),
            other => other.to_string(),
        }
    }

    /// Whether a new request for the same content can succeed.
    const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::IndexUnavailable | Self::Store(_) | Self::Database(_) | Self::Note(_)
        )
    }
}

/// The builder of the releases of the sites.
#[derive(Debug)]
pub struct ReleaseBuilder<I> {
    database: Database,
    index: I,
    notes: NotesBlobReader,
    blobs: SitesBlobs,
}

/// A publication row of the site.
#[derive(Debug)]
struct PublicationRow {
    publication_id: String,
    connection_id: Uuid,
    mode: String,
    included_folders: Vec<String>,
    excluded_folders: Vec<String>,
    included_tags: Vec<String>,
    frontmatter_allow: Vec<String>,
}

impl<I: NotesIndex> ReleaseBuilder<I> {
    /// A builder that reads the index from `index`, note bytes with `notes` within
    /// [`MAX_NOTE_BYTES`], and writes pages and assets under `blob_root`. The bound of `notes`, if
    /// any, is replaced by [`MAX_NOTE_BYTES`].
    #[must_use]
    pub fn new(database: Database, index: I, notes: NotesBlobReader, blob_root: PathBuf) -> Self {
        Self {
            database,
            index,
            notes: notes.with_max_bytes(MAX_NOTE_BYTES),
            blobs: SitesBlobs::new(blob_root),
        }
    }

    /// Build the release of `request.site_id`, and report the operation of the request.
    ///
    /// # Errors
    ///
    /// [`BuildFailure`] when the build makes no release.
    pub async fn build(&self, request: &BuildRequest) -> Result<BuildOutcome, BuildFailure> {
        self.build_job(&BuildJob::from(request)).await
    }

    /// Build the release of `job.site_id`, and report the outcome to each operation of the job.
    ///
    /// # Errors
    ///
    /// [`BuildFailure`] when the build makes no release.
    pub async fn build_job(&self, job: &BuildJob) -> Result<BuildOutcome, BuildFailure> {
        let owner: Option<Uuid> =
            sqlx::query_scalar("select owner_id from sites.sites where site_id = $1")
                .bind(&job.site_id)
                .fetch_optional(self.database.pool())
                .await
                .map_err(BuildFailure::Database)?;
        let owner = owner.ok_or(BuildFailure::SiteNotFound)?;
        let outcome = self.build_site(&job.site_id, owner, job.explicit).await;
        for &operation_id in &job.operation_ids {
            if let Err(error) = report(&self.database, owner, operation_id, &outcome).await {
                // The outcome of the build stays the answer; Platform's stale-operation reaper
                // ends an operation whose report is lost.
                tracing::error!(%error, %operation_id, "the report of a build could not be written");
            }
        }
        outcome
    }

    /// Build `site_id`. A build that is not `explicit` reads only the publications that the current
    /// release contains, so the first release of a publication always comes from a command
    /// (workspace design D6).
    async fn build_site(
        &self,
        site_id: &str,
        owner: Uuid,
        explicit: bool,
    ) -> Result<BuildOutcome, BuildFailure> {
        type Row = (
            String,
            Uuid,
            String,
            Vec<String>,
            Vec<String>,
            Vec<String>,
            Vec<String>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "select publication_id, connection_id, mode, included_folders, excluded_folders, \
             included_tags, frontmatter_allow from sites.publications where site_id = $1 \
             and ($2 or publication_id in (select jsonb_array_elements_text(convert_from( \
             r.document, 'UTF8')::jsonb -> 'publication_ids') from sites.current_releases c \
             join sites.releases r on r.release_id = c.release_id where c.site_id = $1)) \
             order by publication_id",
        )
        .bind(site_id)
        .bind(explicit)
        .fetch_all(self.database.pool())
        .await
        .map_err(BuildFailure::Database)?;
        let rows: Vec<PublicationRow> = rows
            .into_iter()
            .map(
                |(publication_id, connection_id, mode, included, excluded, tags, allow)| {
                    PublicationRow {
                        publication_id,
                        connection_id,
                        mode,
                        included_folders: included,
                        excluded_folders: excluded,
                        included_tags: tags,
                        frontmatter_allow: allow,
                    }
                },
            )
            .collect();
        if rows.is_empty() {
            return Err(BuildFailure::PublicationNotFound);
        }
        let read: Vec<&str> = rows.iter().map(|row| row.publication_id.as_str()).collect();
        let suspended: bool = sqlx::query_scalar(
            "select exists (select 1 from sites.publications where site_id = $1 \
             and publication_id = any($2) and state = 'suspended')",
        )
        .bind(site_id)
        .bind(&read)
        .fetch_one(self.database.pool())
        .await
        .map_err(BuildFailure::Database)?;
        if suspended {
            // The current release keeps the pages of the suspended publication.
            return Err(BuildFailure::PublicationSuspended);
        }
        let mut indexes: BTreeMap<Uuid, Vec<IndexedFile>> = BTreeMap::new();
        for row in &rows {
            if let std::collections::btree_map::Entry::Vacant(vacant) =
                indexes.entry(row.connection_id)
            {
                let files = self
                    .index
                    .files(row.connection_id)
                    .await
                    .map_err(|_| BuildFailure::IndexUnavailable)?;
                vacant.insert(files);
            }
        }
        let site = site_input(site_id, &rows, &indexes)?;
        let sources: Vec<(Uuid, String)> = rows
            .iter()
            .zip(&site.publications)
            .flat_map(|(row, input)| {
                select(&input.publication, input.files)
                    .into_iter()
                    .map(|path| (row.connection_id, path))
            })
            .collect();
        let built = self.render(&site).await?;
        self.store(site_id, owner, &built, &sources).await
    }

    /// Read every selected note with the bound of the reader, and build the release.
    async fn render(&self, site: &SiteInput<'_>) -> Result<Built, BuildFailure> {
        let mut texts = BTreeMap::new();
        for file in selected_notes(site) {
            let bytes = self
                .notes
                .read(&file.revision)
                .await
                .map_err(BuildFailure::Note)?;
            texts.insert(
                file.revision.digest.hex.as_str().to_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            );
        }
        build_release(site, &texts).map_err(BuildFailure::Build)
    }

    /// Store the release unless it is the current one, and make it current.
    async fn store(
        &self,
        site_id: &str,
        owner: Uuid,
        built: &Built,
        sources: &[(Uuid, String)],
    ) -> Result<BuildOutcome, BuildFailure> {
        let document = serde_jcs::to_vec(&built.release).map_err(|error| {
            BuildFailure::Build(BuildError {
                class: ratatoskr_sites_release::PAGE_INVALID,
                detail: error.to_string(),
            })
        })?;
        let release_id = ratatoskr_sites_release::sha256_hex(&document);
        let current: Option<String> =
            sqlx::query_scalar("select release_id from sites.current_releases where site_id = $1")
                .bind(site_id)
                .fetch_optional(self.database.pool())
                .await
                .map_err(BuildFailure::Database)?;
        if current.as_deref() == Some(release_id.as_str()) {
            return Ok(BuildOutcome {
                release_id,
                new_release: false,
            });
        }
        for page in &built.pages {
            let hex = ratatoskr_sites_release::sha256_hex(&page.bytes);
            self.blobs
                .store(&hex, &page.bytes)
                .await
                .map_err(BuildFailure::Store)?;
        }
        for asset in &built.assets {
            let bytes = self.notes.read(asset).await.map_err(BuildFailure::Note)?;
            self.blobs
                .store(asset.digest.hex.as_str(), &bytes)
                .await
                .map_err(BuildFailure::Store)?;
        }
        self.record(site_id, owner, &release_id, &document, built, sources)
            .await?;
        Ok(BuildOutcome {
            release_id,
            new_release: true,
        })
    }

    /// The rows of the release, the current-release pointer and the outbox row of
    /// `sites.release.published.v1`, in one transaction: a reader sees the previous release or this
    /// one, and the event exists exactly when the switch does.
    async fn record(
        &self,
        site_id: &str,
        owner: Uuid,
        release_id: &str,
        document: &[u8],
        built: &Built,
        sources: &[(Uuid, String)],
    ) -> Result<(), BuildFailure> {
        let event_id = Uuid::now_v7();
        let envelope = events::release_published(event_id, owner, release_id, &built.release)
            .map_err(|error| error.to_string())
            .and_then(|envelope| serde_json::to_value(envelope).map_err(|error| error.to_string()))
            .map_err(|detail| {
                BuildFailure::Build(BuildError {
                    class: ratatoskr_sites_release::PAGE_INVALID,
                    detail,
                })
            })?;
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(BuildFailure::Database)?;
        sqlx::query(
            "insert into sites.releases (release_id, site_id, document, page_count, built_at) \
             values ($1, $2, $3, $4, now()) on conflict (release_id) do nothing",
        )
        .bind(release_id)
        .bind(site_id)
        .bind(document)
        .bind(i32::try_from(built.pages.len()).unwrap_or(i32::MAX))
        .execute(&mut *transaction)
        .await
        .map_err(BuildFailure::Database)?;
        for page in &built.release.pages {
            sqlx::query(
                "insert into sites.release_pages (release_id, slug, title, page_digest) \
                 values ($1, $2, $3, $4) on conflict do nothing",
            )
            .bind(release_id)
            .bind(page.slug.as_str())
            .bind(page.title.as_str())
            .bind(page.page_sha256.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(BuildFailure::Database)?;
        }
        for asset in &built.release.assets {
            sqlx::query(
                "insert into sites.release_assets (release_id, digest, media_type, length_bytes) \
                 values ($1, $2, $3, $4) on conflict do nothing",
            )
            .bind(release_id)
            .bind(asset.sha256.as_str())
            .bind(asset.media_type.as_str())
            .bind(i64::try_from(asset.size_bytes).unwrap_or(i64::MAX))
            .execute(&mut *transaction)
            .await
            .map_err(BuildFailure::Database)?;
        }
        for (connection_id, path) in sources {
            sqlx::query(
                "insert into sites.release_sources (release_id, connection_id, path) \
                 values ($1, $2, $3) on conflict do nothing",
            )
            .bind(release_id)
            .bind(connection_id)
            .bind(path)
            .execute(&mut *transaction)
            .await
            .map_err(BuildFailure::Database)?;
        }
        sqlx::query(
            "insert into sites.current_releases (site_id, release_id, made_current_at) \
             values ($1, $2, now()) on conflict (site_id) do update \
             set release_id = excluded.release_id, made_current_at = excluded.made_current_at",
        )
        .bind(site_id)
        .bind(release_id)
        .execute(&mut *transaction)
        .await
        .map_err(BuildFailure::Database)?;
        sqlx::query(
            "insert into sites.outbox (event_id, subject, payload, created_at) \
             values ($1, $2, $3, now())",
        )
        .bind(event_id)
        .bind(events::RELEASE_PUBLISHED)
        .bind(envelope)
        .execute(&mut *transaction)
        .await
        .map_err(BuildFailure::Database)?;
        transaction.commit().await.map_err(BuildFailure::Database)
    }
}

/// Write the report of the operation to the outbox. A report that cannot be built is logged:
/// Platform's stale-operation reaper ends an operation whose report is lost.
pub(crate) async fn report(
    database: &Database,
    owner: Uuid,
    operation_id: Uuid,
    outcome: &Result<BuildOutcome, BuildFailure>,
) -> Result<(), BuildFailure> {
    let event_id = Uuid::now_v7();
    let (class, message) = match outcome {
        Ok(_) => (String::new(), String::new()),
        Err(failure) => (failure.class(), safe_message(&failure.message())),
    };
    let report = match outcome {
        Ok(outcome) => BuildReport::Succeeded {
            release_id: &outcome.release_id,
        },
        Err(failure) => BuildReport::Failed {
            class: &class,
            message: &message,
            retryable: failure.retryable(),
        },
    };
    let envelope = events::build_reported(event_id, owner, operation_id, report)
        .map_err(|error| error.to_string())
        .and_then(|envelope| serde_json::to_value(envelope).map_err(|error| error.to_string()));
    let envelope = match envelope {
        Ok(envelope) => envelope,
        Err(error) => {
            tracing::error!(%error, %operation_id, "a build report could not be built");
            return Ok(());
        }
    };
    sqlx::query(
        "insert into sites.outbox (event_id, subject, payload, created_at) \
         values ($1, $2, $3, now())",
    )
    .bind(event_id)
    .bind(events::OPERATION_REPORTED)
    .bind(envelope)
    .execute(database.pool())
    .await
    .map_err(BuildFailure::Database)?;
    Ok(())
}

/// The longest message of an operation report, in UTF-8 bytes (`SafeMessage`).
const MAX_MESSAGE_BYTES: usize = 1024;

/// `text` as a message that `SafeMessage` accepts: control characters become spaces, and a text
/// over [`MAX_MESSAGE_BYTES`] is cut at a character boundary and ends with `...`. An empty text
/// becomes a fixed message.
fn safe_message(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return "The build made no release.".to_owned();
    }
    if cleaned.len() <= MAX_MESSAGE_BYTES {
        return cleaned.to_owned();
    }
    let mut cut = String::with_capacity(MAX_MESSAGE_BYTES);
    for character in cleaned.chars() {
        if cut.len() + character.len_utf8() > MAX_MESSAGE_BYTES - 3 {
            break;
        }
        cut.push(character);
    }
    cut.push_str("...");
    cut
}

/// The input of the release builder from the publication rows and the indexes.
fn site_input<'a>(
    site_id: &str,
    rows: &[PublicationRow],
    indexes: &'a BTreeMap<Uuid, Vec<IndexedFile>>,
) -> Result<SiteInput<'a>, BuildFailure> {
    let site = SiteId::parse(site_id).map_err(|_| BuildFailure::SiteNotFound)?;
    let mut publications = Vec::with_capacity(rows.len());
    for row in rows {
        let files = indexes
            .get(&row.connection_id)
            .ok_or(BuildFailure::IndexUnavailable)?;
        publications.push(PublicationInput {
            publication: Publication {
                site_id: site.clone(),
                publication_id: PublicationId::parse(&row.publication_id)
                    .map_err(|_| BuildFailure::PublicationInvalid)?,
                mode: match row.mode.as_str() {
                    "all" => PublicationMode::All,
                    _ => PublicationMode::Flagged,
                },
                included_folders: row.included_folders.clone(),
                excluded_folders: row.excluded_folders.clone(),
                included_tags: row.included_tags.clone(),
            },
            connection_id: row.connection_id.to_string(),
            frontmatter_allow: row.frontmatter_allow.clone(),
            files,
        });
    }
    Ok(SiteInput {
        site_id: site,
        publications,
    })
}
