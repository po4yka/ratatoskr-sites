//! The handler of the durables: what a build command, a note change, a vault disconnection and
//! an approval change do (workspace specs `site-publications` and `site-manifest`).
//!
//! The service never publishes a command. A build command of Edge and a note change both become a
//! row of `sites.build_requests`, which the build worker takes.

use std::sync::Arc;

use ratatoskr_event_envelope::{CommandEnvelope, EventEnvelope};
use ratatoskr_notes_contracts::FileChanged;
use ratatoskr_sites_contracts::ReleaseBuildRequested;
use ratatoskr_sites_persistence::Database;
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

    /// A note change: a request row for each site that has a publication over the connection.
    async fn note_change(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Ok(envelope) = EventEnvelope::from_json(payload) else {
            return Ok(Disposition::Reject);
        };
        let Ok(change) = envelope.payload_as::<FileChanged>() else {
            return Ok(Disposition::Reject);
        };
        let sites: Vec<String> = sqlx::query_scalar(
            "select distinct site_id from sites.publications where connection_id = $1 \
             order by site_id",
        )
        .bind(change.connection_id.0)
        .fetch_all(self.database.pool())
        .await?;
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
