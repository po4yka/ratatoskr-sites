//! The handler of the durables: what a build command, a note change, a vault disconnection and
//! an approval change do (workspace specs `site-publications` and `site-manifest`).
//!
//! The service never publishes a command. A build command of Edge and a note change both become a
//! row of `sites.build_requests`, which the build worker takes.

use std::path::PathBuf;
use std::sync::Arc;

use ratatoskr_event_envelope::{CommandEnvelope, EventEnvelope};
use ratatoskr_notes_contracts::{FileChanged, VaultDisconnected, VaultPath};
use ratatoskr_sites_contracts::{
    ApprovalState, PublicationId, ReleaseBuildRequested, SiteApprovalChanged, SiteId,
};
use ratatoskr_sites_deploys::{SiteProjection, write_projection};
use ratatoskr_sites_persistence::Database;
use ratatoskr_sites_publications::{Publication, PublicationMode, select};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::consumer::{self, BusTarget, Disposition, Durable, Durables, Inbound};
use crate::events;
use crate::notes_index::{IndexError, NotesIndex};
use crate::release_build::{BuildFailure, report};
use crate::scope::{self, ScopeError};
use crate::servable::Servable;
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
    /// The gateway projection could not be written.
    #[error("the gateway projection could not be written")]
    Projection(#[source] std::io::Error),
}

/// What the consumers, the build worker and the routes share.
#[derive(Debug, Clone)]
pub struct Shared {
    /// The database.
    pub database: Database,
    /// The servable sites.
    pub servable: Arc<Servable>,
    /// The sites tree where the approval consumer writes the gateway projection of each site
    /// (SITE-026); `None` writes none.
    pub sites_tree: Option<PathBuf>,
}

/// The durables to consume and where.
#[derive(Debug, Clone, Copy)]
pub struct Consumers<'a> {
    /// The server and the identity.
    pub target: &'a BusTarget,
    /// The specs of the four durables.
    pub specs: &'a Durables,
    /// The durables to consume.
    pub durables: &'a [Durable],
}

/// Start the consumers of `durables` and the build worker, until `stop`. The worker builds with
/// `builds`, and the handler reads the notes index from `index`.
pub fn start<I: NotesIndex + 'static, B: SiteBuilds>(
    shared: &Shared,
    index: I,
    builds: B,
    consumers: &Consumers<'_>,
    stop: &CancellationToken,
) -> Vec<JoinHandle<()>> {
    let (queue, worker) = build_queue(shared.clone(), builds);
    shared.servable.attach(queue.clone());
    let triggers = Arc::new(Triggers::new(shared.clone(), index, queue));
    let mut tasks = consumer::spawn(
        consumers.target,
        consumers.specs,
        consumers.durables,
        &triggers,
        stop,
    );
    tasks.push(tokio::spawn(worker.run(stop.clone())));
    tasks
}

/// The handler of the four durables.
#[derive(Debug)]
pub struct Triggers<I> {
    database: Database,
    servable: Arc<Servable>,
    sites_tree: Option<PathBuf>,
    index: I,
    queue: BuildQueue,
}

impl<I: NotesIndex + 'static> Triggers<I> {
    /// A handler that reads the notes index from `index` and wakes the worker of `queue`.
    #[must_use]
    pub fn new(shared: Shared, index: I, queue: BuildQueue) -> Self {
        Self {
            database: shared.database,
            servable: shared.servable,
            sites_tree: shared.sites_tree,
            index,
            queue,
        }
    }

    /// A build command of Edge: one request row, keyed by the command id. The inbox records the
    /// command, so a redelivery after the build adds nothing. A command for a site that does not
    /// exist fails its operation at once.
    async fn command(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Some((command_id, owner, request)) = parse_command(payload) else {
            return Ok(Disposition::Reject);
        };
        let site_id = request.site_id.as_str();
        let operation_id = request.operation_id.0;
        let known: bool =
            sqlx::query_scalar("select exists (select 1 from sites.sites where site_id = $1)")
                .bind(site_id)
                .fetch_one(self.database.pool())
                .await?;
        let mut transaction = self.database.pool().begin().await?;
        let fresh = sqlx::query(
            "insert into sites.inbox (message_id, subject, consumed_at) values ($1, $2, now()) \
             on conflict (message_id) do nothing",
        )
        .bind(command_id)
        .bind("cmd.sites.release.build_requested.v1")
        .execute(&mut *transaction)
        .await?;
        if fresh.rows_affected() == 0 {
            // A redelivery of a command that was applied: its build ran or waits already.
            return Ok(Disposition::Done);
        }
        if known {
            sqlx::query(
                "insert into sites.build_requests (request_id, site_id, origin, operation_id, \
                 requested_at) values ($1, $2, 'command', $3, now()) \
                 on conflict (request_id) do nothing",
            )
            .bind(command_id)
            .bind(site_id)
            .bind(operation_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        if known {
            self.queue.wake(site_id);
        } else {
            let outcome = Err(BuildFailure::SiteNotFound);
            report(&self.database, owner, operation_id, &outcome).await?;
        }
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
        let Some(drafts) = self.drafts_of(&change).await? else {
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
        for (event_id, envelope) in drafts {
            sqlx::query(
                "insert into sites.outbox (event_id, subject, payload, created_at) \
                 values ($1, $2, $3, now())",
            )
            .bind(event_id)
            .bind(events::DRAFT_CHANGED)
            .bind(envelope)
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
                        // A removed connection has no file that a publication selects now.
                        Err(IndexError::ConnectionNotFound) => files = Some(Vec::new()),
                        Err(IndexError::Unavailable) => return Ok(None),
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

impl<I: NotesIndex + 'static> Triggers<I> {
    /// The draft change events of `change`: one for each site whose effective publications select
    /// the note now, in either release mode (SITE-027, workspace design D5). `None` when the notes
    /// index is unavailable. A deleted note is selected by no publication, so it makes none.
    async fn drafts_of(
        &self,
        change: &FileChanged,
    ) -> Result<Option<Vec<(Uuid, serde_json::Value)>>, ApplyError> {
        let sites: Vec<(String, Uuid)> = sqlx::query_as(
            "select distinct p.site_id, s.owner_id from sites.publications p \
             join sites.sites s on s.site_id = p.site_id \
             where p.connection_id = $1 and p.state = 'active' order by p.site_id",
        )
        .bind(change.connection_id.0)
        .fetch_all(self.database.pool())
        .await?;
        let connection_id = change.connection_id.0;
        let path = change.path.as_str();
        let mut drafts = Vec::new();
        for (site_id, owner_id) in sites {
            match scope::in_scope(&self.database, &self.index, &site_id, connection_id, path).await
            {
                Ok(Some(_)) => {}
                Ok(None) => continue,
                Err(ScopeError::Unavailable) => return Ok(None),
                Err(ScopeError::Database(error)) => return Err(ApplyError::Database(error)),
            }
            let publications = scope::effective_publications(&self.database, &site_id).await?;
            let slug =
                scope::released_slug(&self.database, &site_id, &publications, connection_id, path)
                    .await?;
            let source_ref =
                scope::source_ref(&self.database, &site_id, connection_id, path).await?;
            let event_id = Uuid::now_v7();
            let built = events::draft_changed(
                event_id,
                owner_id,
                &site_id,
                &source_ref,
                change.revision.as_str(),
                slug.as_deref(),
            )
            .map_err(|error| error.to_string())
            .and_then(|envelope| serde_json::to_value(envelope).map_err(|error| error.to_string()));
            match built {
                Ok(envelope) => drafts.push((event_id, envelope)),
                Err(error) => tracing::warn!(%error, "a draft change event could not be built"),
            }
        }
        Ok(Some(drafts))
    }

    /// A vault disconnection: every publication over the connection becomes `suspended`. The
    /// current release of each site stays current (workspace spec `site-publications`).
    async fn disconnection(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Ok(envelope) = EventEnvelope::from_json(payload) else {
            return Ok(Disposition::Reject);
        };
        let Ok(disconnected) = envelope.payload_as::<VaultDisconnected>() else {
            return Ok(Disposition::Reject);
        };
        let mut transaction = self.database.pool().begin().await?;
        let fresh = sqlx::query(
            "insert into sites.inbox (message_id, subject, consumed_at) values ($1, $2, now()) \
             on conflict (message_id) do nothing",
        )
        .bind(envelope.event_id.0)
        .bind("evt.notes.vault.disconnected.v1")
        .execute(&mut *transaction)
        .await?;
        if fresh.rows_affected() > 0 {
            sqlx::query(
                "update sites.publications set state = 'suspended' where connection_id = $1",
            )
            .bind(disconnected.connection_id.0)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(Disposition::Done)
    }
}

impl<I: NotesIndex + 'static> Triggers<I> {
    /// An approval change of Platform: the read projection keeps the last approved digest, its
    /// capabilities, its hostname and the state of each site (workspace spec `site-manifest`). An
    /// older change than the stored one changes nothing, so a redelivery out of order is safe.
    async fn approval(&self, payload: &[u8]) -> Result<Disposition, ApplyError> {
        let Ok(envelope) = EventEnvelope::from_json(payload) else {
            return Ok(Disposition::Reject);
        };
        let Ok(change) = envelope.payload_as::<SiteApprovalChanged>() else {
            return Ok(Disposition::Reject);
        };
        let state = match change.state {
            ApprovalState::Approved => "approved",
            _ => "withdrawn",
        };
        let capabilities: Vec<&str> = change
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect();
        let mut transaction = self.database.pool().begin().await?;
        let fresh = sqlx::query(
            "insert into sites.inbox (message_id, subject, consumed_at) values ($1, $2, now()) \
             on conflict (message_id) do nothing",
        )
        .bind(envelope.event_id.0)
        .bind("evt.platform.site_approval.changed.v1")
        .execute(&mut *transaction)
        .await?;
        if fresh.rows_affected() > 0 {
            sqlx::query(
                "insert into sites.approval_projection (site_id, approved_digest, state, \
                 capabilities, hostname, changed_at, github_repository) \
                 values ($1, $2, $3, $4, $5, $6::timestamptz, $7) \
                 on conflict (site_id) do update set approved_digest = excluded.approved_digest, \
                 state = excluded.state, capabilities = excluded.capabilities, \
                 hostname = excluded.hostname, changed_at = excluded.changed_at, \
                 github_repository = excluded.github_repository \
                 where sites.approval_projection.changed_at <= excluded.changed_at",
            )
            .bind(change.site_id.as_str())
            .bind(change.manifest_sha256.as_str())
            .bind(state)
            .bind(&capabilities)
            .bind(change.hostname.as_str())
            .bind(envelope.occurred_at.to_string())
            .bind(change.github_repository.as_ref().map(sqlx::types::Json))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.servable
            .refresh(&self.database, change.site_id.as_str())
            .await?;
        self.project(change.site_id.as_str()).await?;
        Ok(Disposition::Done)
    }

    /// Rewrite the gateway projection `site.json` of `site_id` from the approval projection, which
    /// an older event does not change (SITE-026, workspace design D2).
    async fn project(&self, site_id: &str) -> Result<(), ApplyError> {
        let Some(root) = self.sites_tree.clone() else {
            return Ok(());
        };
        let row: Option<(String, String)> = sqlx::query_as(
            "select state, hostname from sites.approval_projection where site_id = $1",
        )
        .bind(site_id)
        .fetch_optional(self.database.pool())
        .await?;
        let Some((state, hostname)) = row else {
            return Ok(());
        };
        let projection = SiteProjection::of(site_id, state == "approved", &hostname);
        tokio::task::spawn_blocking(move || write_projection(&root, &projection))
            .await
            .map_err(|_| ApplyError::Projection(std::io::Error::other("the write stopped")))?
            .map_err(ApplyError::Projection)
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
            Durable::VaultDisconnections => self.disconnection(payload).await,
            Durable::Approvals => self.approval(payload).await,
        };
        applied.unwrap_or_else(|error| {
            tracing::warn!(%error, ?durable, "a message could not be applied; it comes again");
            Disposition::Retry
        })
    }
}
