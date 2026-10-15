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

/// The worker that builds the requested sites.
#[derive(Debug)]
pub struct BuildWorker<B> {
    database: Database,
    servable: Arc<Servable>,
    builds: B,
    wake: mpsc::Receiver<String>,
}

/// A queue and the worker that it wakes.
pub fn build_queue<B: SiteBuilds>(shared: Shared, builds: B) -> (BuildQueue, BuildWorker<B>) {
    let (sender, receiver) = mpsc::channel(WAKE_CAPACITY);
    (
        BuildQueue { wake: sender },
        BuildWorker {
            database: shared.database,
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

    /// Build `site_id` until it has no request row. One build takes every row that is there when
    /// it starts, and deletes exactly those rows after it, so the requests that arrive while a
    /// build runs make one more build (workspace design D6).
    async fn drain(&self, site_id: &str) {
        loop {
            let rows: Result<Vec<(Uuid, String, Option<Uuid>)>, sqlx::Error> = sqlx::query_as(
                "select request_id, origin, operation_id from sites.build_requests \
                 where site_id = $1 order by requested_at, request_id",
            )
            .bind(site_id)
            .fetch_all(self.database.pool())
            .await;
            let rows = match rows {
                Ok(rows) if rows.is_empty() => return,
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, site_id, "the build requests could not be read");
                    return;
                }
            };
            let job = BuildJob {
                site_id: site_id.to_owned(),
                operation_ids: rows
                    .iter()
                    .filter_map(|(_, _, operation)| *operation)
                    .collect(),
                explicit: rows.iter().any(|(_, origin, _)| origin == "command"),
            };
            match self.builds.build(&job).await {
                Ok(outcome) if outcome.new_release => {
                    if let Err(error) = self.servable.refresh(&self.database, site_id).await {
                        tracing::warn!(%error, site_id, "the servable sites could not be read");
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::info!(%error, class = %error.class(), site_id, "a build made no release");
                }
            }
            let taken: Vec<Uuid> = rows.iter().map(|(request, _, _)| *request).collect();
            if let Err(error) =
                sqlx::query("delete from sites.build_requests where request_id = any($1)")
                    .bind(&taken)
                    .execute(self.database.pool())
                    .await
            {
                tracing::warn!(%error, site_id, "the build requests could not be removed");
                return;
            }
        }
    }
}
