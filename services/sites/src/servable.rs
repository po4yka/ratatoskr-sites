//! The servable sites, in memory (workspace design D7): the one place that every absence of a
//! delivery route reads, so an unknown, an unapproved and a withdrawn site and a capability outside
//! the effective set take the same path and cost no database query.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock, RwLock};

use ratatoskr_sites_contracts::SiteCapability;
use ratatoskr_sites_persistence::Database;
#[cfg(feature = "test-support")]
use tokio::sync::Notify;

use crate::effective::effective_set;
use crate::worker::{BuildQueue, request_removal_build};

/// What a servable site serves now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serving {
    /// The current release.
    pub release_id: String,
    /// The public capabilities of the effective set.
    pub capabilities: BTreeSet<SiteCapability>,
    /// The publications of the effective set, in ascending order. The delivery serves a page, an
    /// asset or a search hit only when one of its publications is here.
    pub publications: Vec<String>,
    /// The `ETag` of the release, page and search answers: it changes with the release and with
    /// the effective publications.
    pub tag: String,
}

/// The servable sites. Shared by the routes, the consumers and the build worker.
#[derive(Debug, Default)]
pub struct Servable {
    sites: RwLock<HashMap<String, Serving>>,
    /// Held across the read and the write of each refresh, so a refresh that read the database
    /// earlier cannot write after one that read it later.
    refreshing: tokio::sync::Mutex<()>,
    /// The build worker, woken for a removal build.
    queue: OnceLock<BuildQueue>,
    #[cfg(feature = "test-support")]
    pause: std::sync::Mutex<Option<RefreshPause>>,
}

/// The `ETag` value of the release `release_id` served for `publications`: the release id, and the
/// start of the SHA-256 of the publication list.
fn tag_of(release_id: &str, publications: &[String]) -> String {
    let list = ratatoskr_sites_release::sha256_hex(publications.join("\n").as_bytes());
    format!("\"{release_id}.{}\"", list.get(..16).unwrap_or_default())
}

#[cfg(feature = "test-support")]
/// A seam for the tests of the refresh: the next refresh stops between its read and its write,
/// tells [`RefreshPause::reached`], and waits for [`RefreshPause::release`].
#[derive(Debug, Clone, Default)]
pub struct RefreshPause {
    /// Notified when the refresh has read the database and waits.
    pub reached: Arc<Notify>,
    /// Notify it to let the refresh write.
    pub release: Arc<Notify>,
}

impl Servable {
    /// The servable sites as the database holds them now.
    ///
    /// # Errors
    ///
    /// The query error when the sites cannot be read.
    pub async fn load(database: &Database) -> Result<Arc<Self>, sqlx::Error> {
        let servable = Arc::new(Self::default());
        let sites: Vec<String> =
            sqlx::query_scalar("select site_id from sites.sites order by site_id")
                .fetch_all(database.pool())
                .await?;
        for site_id in sites {
            servable.refresh(database, &site_id).await?;
        }
        Ok(servable)
    }

    /// Wake `queue` for a removal build when a refresh finds that publications left the effective
    /// set of a site.
    pub fn attach(&self, queue: BuildQueue) {
        let _ = self.queue.set(queue);
    }

    /// Wake the build worker for `site_id`, when one is attached.
    pub fn wake(&self, site_id: &str) {
        if let Some(queue) = self.queue.get() {
            queue.wake(site_id);
        }
    }

    /// Stop the next refresh between its read and its write. Built only with the feature
    /// `test-support`.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn pause_next_refresh(&self) -> RefreshPause {
        let pause = RefreshPause::default();
        if let Ok(mut held) = self.pause.lock() {
            *held = Some(pause.clone());
        }
        pause
    }

    /// Read `site_id` again from the database. Refreshes run one at a time: the read and the write
    /// of one refresh are never split by another refresh, so the last refresh to read writes last.
    ///
    /// # Errors
    ///
    /// The query error when the site cannot be read; the set keeps what it had.
    pub async fn refresh(&self, database: &Database, site_id: &str) -> Result<(), sqlx::Error> {
        let _one_at_a_time = self.refreshing.lock().await;
        let effective = effective_set(database, site_id).await?;
        let release: Option<String> =
            sqlx::query_scalar("select release_id from sites.current_releases where site_id = $1")
                .bind(site_id)
                .fetch_optional(database.pool())
                .await?;
        let capabilities: BTreeSet<SiteCapability> = effective
            .capabilities
            .into_iter()
            .filter(|capability| capability.is_public())
            .collect();
        let serving = release
            .filter(|_| !capabilities.is_empty())
            .map(|release_id| {
                let publications: Vec<String> = effective.publications.into_iter().collect();
                let tag = tag_of(&release_id, &publications);
                Serving {
                    release_id,
                    capabilities,
                    publications,
                    tag,
                }
            });
        #[cfg(feature = "test-support")]
        {
            let pause = self.pause.lock().ok().and_then(|mut held| held.take());
            if let Some(pause) = pause {
                pause.reached.notify_one();
                pause.release.notified().await;
            }
        }
        let shrunk = self.sites.read().ok().is_some_and(|sites| {
            sites.get(site_id).is_some_and(|before| {
                serving.as_ref().is_some_and(|now| {
                    before
                        .publications
                        .iter()
                        .any(|publication| !now.publications.contains(publication))
                })
            })
        });
        if shrunk && request_removal_build(database, site_id).await? {
            self.wake(site_id);
        }
        if let Ok(mut sites) = self.sites.write() {
            match serving {
                Some(serving) => sites.insert(site_id.to_owned(), serving),
                None => sites.remove(site_id),
            };
        }
        Ok(())
    }

    /// What `site_id` serves for `capability`, or `None` for every kind of absence.
    #[must_use]
    pub fn serving(&self, site_id: &str, capability: SiteCapability) -> Option<Serving> {
        let sites = self.sites.read().ok()?;
        sites
            .get(site_id)
            .filter(|serving| serving.capabilities.contains(&capability))
            .cloned()
    }
}
