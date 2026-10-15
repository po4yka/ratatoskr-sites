//! The handler of the durables: what a build command, a note change, a vault disconnection and
//! an approval change do (workspace specs `site-publications` and `site-manifest`).
//!
//! The service never publishes a command. A build command of Edge and a note change both become a
//! row of `sites.build_requests`, which the build worker takes.

use std::sync::Arc;

use ratatoskr_event_envelope::{CommandEnvelope, EventEnvelope};
use ratatoskr_notes_contracts::{FileChanged, VaultPath};
use ratatoskr_sites_contracts::{PublicationId, ReleaseBuildRequested, SiteId};
use ratatoskr_sites_persistence::Database;
use ratatoskr_sites_publications::{Publication, PublicationMode, select};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::consumer::{self, BusTarget, Disposition, Durable, Durables, Inbound};
use crate::notes_index::NotesIndex;
use crate::release_build::{BuildFailure, report};
use crate::worker::{BuildQueue, SiteBuilds, build_queue};

/// Why a message could not be applied. It is delivered again.
#[derive(Debug, thiserror::Error)]
enum ApplyError {
    /// A query failed.
    #[error("a query failed")]
    Database(#[from] sqlx::Error),
    /// The report of an operation could not be written.
    #[error("an operation report could not be written")]
    Report(#[from] BuildFailure),
}

/// Start the consumers of `durables` and the build worker, until `stop`. The worker builds with
/// `builds`, and the handler reads the notes index from `index`.
pub fn start<I: NotesIndex + 'static, B: SiteBuilds>(
    database: &Database,
    index: I,
    builds: B,
    target: &BusTarget,
    specs: &Durables,
    durables: &[Durable],
    stop: &CancellationToken,
) -> Vec<JoinHandle<()>> {
    let (queue, worker) = build_queue(database.clone(), builds);
    let triggers = Arc::new(Triggers::new(database.clone(), index, queue));
    let mut tasks = consumer::spawn(target, specs, durables, &triggers, stop);
    tasks.push(tokio::spawn(worker.run(stop.clone())));
    tasks
}

/// The handler of the four durables.
#[derive(Debug)]
pub struct Triggers<I> {
    database: Database,
    index: I,
    queue: BuildQueue,
}

impl<I: NotesIndex + 'static> Triggers<I> {
    /// A handler that reads the notes index from `index` and wakes the worker of `queue`.
    #[must_use]
    pub const fn new(database: Database, index: I, queue: BuildQueue) -> Self {
        Self {
            database,
            index,
            queue,
        }
    }

    /// A build command of Edge: one request row, keyed by the command id, so that a redelivery
    /// adds nothing. A command for a site that does not exist fails its operation at once.
    async fn command(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Some((command_id, owner, request)) = parse_command(payload) else {
            return Ok(Disposition::Reject);
        };
        let site_id = request.site_id.as_str();
        let operation_id = request.operation_id.0;
        let inserted = sqlx::query(
            "insert into sites.build_requests (request_id, site_id, origin, operation_id, \
             requested_at) select $1, site_id, 'command', $2, now() from sites.sites \
             where site_id = $3 on conflict (request_id) do nothing",
        )
        .bind(command_id)
        .bind(operation_id)
        .bind(site_id)
        .execute(self.database.pool())
        .await?;
        if inserted.rows_affected() == 0 {
            let known: bool =
                sqlx::query_scalar("select exists (select 1 from sites.sites where site_id = $1)")
                    .bind(site_id)
                    .fetch_one(self.database.pool())
                    .await?;
            if !known {
                let outcome = Err(BuildFailure::SiteNotFound);
                report(&self.database, owner, operation_id, &outcome).await?;
                return Ok(Disposition::Done);
            }
        }
        self.queue.wake(site_id);
        Ok(Disposition::Done)
    }

    /// A note change: a request row for each site whose build it requests ([`Self::sites_to_build`]).
    async fn note_change(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Ok(envelope) = EventEnvelope::from_json(payload) else {
            return Ok(Disposition::Reject);
        };
        let Ok(change) = envelope.payload_as::<FileChanged>() else {
            return Ok(Disposition::Reject);
        };
        let Some(sites) = self.sites_to_build(&change).await? else {
            return Ok(Disposition::Retry);
        };
        let mut transaction = self.database.pool().begin().await?;
        let fresh = sqlx::query(
            "insert into sites.inbox (message_id, subject, consumed_at) values ($1, $2, now()) \
             on conflict (message_id) do nothing",
        )
        .bind(envelope.event_id.0)
        .bind("evt.notes.file.changed.v1")
        .execute(&mut *transaction)
        .await?;
        if fresh.rows_affected() == 0 {
            return Ok(Disposition::Done);
        }
        for site_id in &sites {
            sqlx::query(
                "insert into sites.build_requests (request_id, site_id, origin, operation_id, \
                 requested_at) values ($1, $2, 'file_change', null, now())",
            )
            .bind(Uuid::now_v7())
            .bind(site_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        for site_id in &sites {
            self.queue.wake(site_id);
        }
        Ok(Disposition::Done)
    }
}

impl<I: NotesIndex + 'static> Triggers<I> {
    /// The sites whose build `change` requests, or `None` when the notes index is unavailable.
    ///
    /// A publication takes part when it is automatic and active, and the current release of its
    /// site contains it: the first release of a publication always comes from a command. The
    /// change requests a build when the publication selects the file now, or selected it (or its
    /// path before a rename) in the current release.
    async fn sites_to_build(
        &self,
        change: &FileChanged,
    ) -> Result<Option<Vec<String>>, ApplyError> {
        type Row = (
            String,
            String,
            String,
            Vec<String>,
            Vec<String>,
            Vec<String>,
            bool,
        );
        let paths: Vec<&str> = std::iter::once(change.path.as_str())
            .chain(change.previous_path.as_ref().map(VaultPath::as_str))
            .collect();
        let rows: Vec<Row> = sqlx::query_as(
            "select p.site_id, p.publication_id, p.mode, p.included_folders, \
             p.excluded_folders, p.included_tags, exists (select 1 from sites.release_sources s \
             where s.release_id = c.release_id and s.connection_id = p.connection_id \
             and s.path = any($2)) from sites.publications p \
             join sites.current_releases c on c.site_id = p.site_id \
             join sites.releases r on r.release_id = c.release_id \
             where p.connection_id = $1 and p.release_mode = 'automatic' and p.state = 'active' \
             and (convert_from(r.document, 'UTF8')::jsonb -> 'publication_ids') ? p.publication_id \
             order by p.site_id, p.publication_id",
        )
        .bind(change.connection_id.0)
        .bind(&paths)
        .fetch_all(self.database.pool())
        .await?;
        let mut files = None;
        let mut sites: Vec<String> = Vec::new();
        for (site_id, publication_id, mode, included, excluded, tags, selected_before) in rows {
            if sites.contains(&site_id) {
                continue;
            }
            let selected_now = if selected_before {
                false
            } else {
                if files.is_none() {
                    match self.index.files(change.connection_id.0).await {
                        Ok(read) => files = Some(read),
                        Err(_) => return Ok(None),
                    }
                }
                let (Ok(site), Ok(publication_id)) = (
                    SiteId::parse(&site_id),
                    PublicationId::parse(&publication_id),
                ) else {
                    continue;
                };
                let publication = Publication {
                    site_id: site,
                    publication_id,
                    mode: if mode == "all" {
                        PublicationMode::All
                    } else {
                        PublicationMode::Flagged
                    },
                    included_folders: included,
                    excluded_folders: excluded,
                    included_tags: tags,
                };
                select(&publication, files.as_deref().unwrap_or_default())
                    .contains(change.path.as_str())
            };
            if selected_before || selected_now {
                sites.push(site_id);
            }
        }
        Ok(Some(sites))
    }
}

/// The command id, the owner and the payload of a build command.
fn parse_command(payload: &[u8]) -> Option<(Uuid, Uuid, ReleaseBuildRequested)> {
    let command = CommandEnvelope::from_json(payload).ok()?;
    let request = command.payload_as::<ReleaseBuildRequested>().ok()?;
    let owner = command.tenant_id?.user_id().0;
    Some((command.command_id.0, owner, request))
}

impl<I: NotesIndex + 'static> Inbound for Triggers<I> {
    async fn handle(&self, durable: Durable, payload: &[u8]) -> Disposition {
        let applied = match durable {
            Durable::ReleaseBuilds => self.command(payload).await,
            Durable::NoteChanges => self.note_change(payload).await,
            Durable::VaultDisconnections | Durable::Approvals => Ok(Disposition::Retry),
        };
        applied.unwrap_or_else(|error| {
            tracing::warn!(%error, ?durable, "a message could not be applied; it comes again");
            Disposition::Retry
        })
    }
}
