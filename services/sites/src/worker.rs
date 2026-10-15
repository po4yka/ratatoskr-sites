//! The build worker (workspace design D6): it takes the build requests of `sites.build_requests`
//! and builds the sites that they name, one build at a time.
//!
//! A handler writes a request row and then wakes the worker for its site through a bounded channel.
//! When the channel is full the wake-up is dropped; the worker finds the row on its next sweep.

use std::future::Future;
use std::time::Duration;

use ratatoskr_sites_persistence::Database;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use std::sync::Arc;

use crate::notes_index::NotesIndex;
use crate::release_build::{BuildFailure, BuildJob, BuildOutcome, ReleaseBuilder};
use crate::servable::Servable;
use crate::triggers::Shared;

/// How many wake-ups wait for the worker at most.
const WAKE_CAPACITY: usize = 256;

/// How many times the worker tries a build that fails for a transient reason (the notes index, the
/// database, the blob store) before it reports the failure.
const MAX_ATTEMPTS: i32 = 3;

/// How long the worker waits before it tries such a build again.
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// How often the worker reads every site that has a request, also without a wake-up.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// What builds a site.
pub trait SiteBuilds: Send + Sync + 'static {
    /// Build `job`, and report the outcome to its operations.
    fn build(
        &self,
        job: &BuildJob,
    ) -> impl Future<Output = Result<BuildOutcome, BuildFailure>> + Send;
}

impl<I: NotesIndex + 'static> SiteBuilds for ReleaseBuilder<I> {
    async fn build(&self, job: &BuildJob) -> Result<BuildOutcome, BuildFailure> {
        self.build_job(job).await
    }
}

/// The handle that wakes the worker for a site. Clones share the worker.
#[derive(Debug, Clone)]
pub struct BuildQueue {
    wake: mpsc::Sender<String>,
}

impl BuildQueue {
    /// Tell the worker that `site_id` has a new request row.
    pub fn wake(&self, site_id: &str) {
        if self.wake.try_send(site_id.to_owned()).is_err() {
            tracing::debug!(
                site_id,
                "the build worker is busy; its sweep finds the request"
            );
        }
    }
}

/// Request a build of `site_id` because a publication left its effective set or changed (workspace
/// spec `site-manifest`, "a removal stops at once"): the links, backlinks and titles of the removed
/// pages leave the pages that stay. It is written only for a site with a current release, in each
/// release mode, and only when no removal request waits that no build has taken yet; a request that
/// a running build took does not count, so a removal during a build makes one more build. A retry
/// after a transient failure releases the rows it keeps, so two removal requests can wait at once;
/// the next build takes both. A removal build reads only the publications of the current release
/// that stay in the effective set. Returns whether a request was written.
///
/// # Errors
///
/// The query error.
pub(crate) async fn request_removal_build(
    database: &Database,
    site_id: &str,
) -> Result<bool, sqlx::Error> {
    let written = sqlx::query(
        "insert into sites.build_requests (request_id, site_id, origin, operation_id, \
         requested_at) select $1, site_id, 'removal', null, now() from sites.current_releases \
         where site_id = $2 and not exists (select 1 from sites.build_requests \
         where site_id = $2 and origin = 'removal' and taken_at is null)",
    )
    .bind(Uuid::now_v7())
    .bind(site_id)
    .execute(database.pool())
    .await?;
    Ok(written.rows_affected() > 0)
}

/// The worker that builds the requested sites.
#[derive(Debug)]
pub struct BuildWorker<B> {
    database: Database,
    /// The sender of the wake-ups, for a retry after a transient failure.
    retry: mpsc::Sender<String>,
    servable: Arc<Servable>,
    builds: B,
    wake: mpsc::Receiver<String>,
}

/// A queue and the worker that it wakes.
pub fn build_queue<B: SiteBuilds>(shared: Shared, builds: B) -> (BuildQueue, BuildWorker<B>) {
    let (sender, receiver) = mpsc::channel(WAKE_CAPACITY);
    (
        BuildQueue {
            wake: sender.clone(),
        },
        BuildWorker {
            database: shared.database,
            retry: sender,
            servable: shared.servable,
            builds,
            wake: receiver,
        },
    )
}

impl<B: SiteBuilds> BuildWorker<B> {
    /// Build the requested sites until `stop`. A build that runs when `stop` comes ends first.
    pub async fn run(mut self, stop: CancellationToken) {
        let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                woken = self.wake.recv() => match woken {
                    Some(site_id) => self.drain(&site_id).await,
                    None => return,
                },
                _ = sweep.tick() => self.sweep().await,
            }
        }
    }

    /// Build every site that has a request row.
    async fn sweep(&self) {
        let sites: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(
            "select distinct site_id from sites.build_requests order by site_id",
        )
        .fetch_all(self.database.pool())
        .await;
        match sites {
            Ok(sites) => {
                for site_id in sites {
                    self.drain(&site_id).await;
                }
            }
            Err(error) => tracing::warn!(%error, "the build requests could not be read"),
        }
    }

    /// Wake the worker for `site_id` after [`RETRY_DELAY`].
    fn retry_later(&self, site_id: &str) {
        let wake = self.retry.clone();
        let site_id = site_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(RETRY_DELAY).await;
            let _ = wake.try_send(site_id);
        });
    }

    /// Build `site_id` until it has no request row. One build takes every row that is there when
    /// it starts, and deletes exactly those rows after it, so the requests that arrive while a
    /// build runs make one more build (workspace design D6).
    ///
    /// Each row counts its own attempts, before the build: when the count cannot be written,
    /// nothing is built. A build that fails for a transient reason deletes the rows that reached
    /// [`MAX_ATTEMPTS`], reports the failure to their operations only, and is tried again for the
    /// other rows after [`RETRY_DELAY`].
    async fn drain(&self, site_id: &str) {
        type Row = (Uuid, String, Option<Uuid>, i32);
        loop {
            let rows: Result<Vec<Row>, sqlx::Error> = sqlx::query_as(
                "update sites.build_requests set attempts = attempts + 1, taken_at = now() \
                 where site_id = $1 returning request_id, origin, operation_id, attempts",
            )
            .bind(site_id)
            .fetch_all(self.database.pool())
            .await;
            let rows = match rows {
                Ok(rows) if rows.is_empty() => return,
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, site_id, "the build requests could not be counted; nothing is built");
                    return;
                }
            };
            let job = BuildJob {
                site_id: site_id.to_owned(),
                operation_ids: rows.iter().filter_map(|row| row.2).collect(),
                explicit: rows.iter().any(|row| row.1 == "command"),
                from_release: rows.iter().all(|row| row.1 == "removal"),
                retried: rows
                    .iter()
                    .filter(|row| row.3 < MAX_ATTEMPTS)
                    .filter_map(|row| row.2)
                    .collect(),
            };
            let mut taken: Vec<Uuid> = rows.iter().map(|row| row.0).collect();
            let mut again = false;
            match self.builds.build(&job).await {
                // Also a build that made the current release again: its provenance can have changed.
                Ok(_) => {
                    if let Err(error) = self.servable.refresh(&self.database, site_id).await {
                        tracing::warn!(%error, site_id, "the servable sites could not be read");
                    }
                }
                Err(error) if error.retryable() => {
                    taken = rows
                        .iter()
                        .filter(|row| row.3 >= MAX_ATTEMPTS)
                        .map(|row| row.0)
                        .collect();
                    again = taken.len() < rows.len();
                    tracing::info!(%error, class = %error.class(), site_id, again, "a build failed for a transient reason");
                }
                Err(error) => {
                    tracing::info!(%error, class = %error.class(), site_id, "a build made no release");
                }
            }
            if let Err(error) =
                sqlx::query("delete from sites.build_requests where request_id = any($1)")
                    .bind(&taken)
                    .execute(self.database.pool())
                    .await
            {
                tracing::warn!(%error, site_id, "the build requests could not be removed");
                return;
            }
            if again {
                // The rows that are tried again wait like new ones.
                if let Err(error) = sqlx::query(
                    "update sites.build_requests set taken_at = null \
                     where site_id = $1 and taken_at is not null",
                )
                .bind(site_id)
                .execute(self.database.pool())
                .await
                {
                    tracing::warn!(%error, site_id, "the build requests could not be released");
                }
                self.retry_later(site_id);
                return;
            }
        }
    }
}
