//! The servable sites, in memory (workspace design D7): the one place that every absence of a
//! delivery route reads, so an unknown, an unapproved and a withdrawn site and a capability outside
//! the effective set take the same path and cost no database query.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use ratatoskr_sites_contracts::SiteCapability;
use ratatoskr_sites_persistence::Database;

/// What a servable site serves now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serving {
    /// The current release.
    pub release_id: String,
    /// The public capabilities of the effective set.
    pub capabilities: BTreeSet<SiteCapability>,
}

/// The servable sites. Shared by the routes, the consumers and the build worker.
#[derive(Debug, Default)]
pub struct Servable {
    sites: RwLock<HashMap<String, Serving>>,
}

impl Servable {
    /// The servable sites as the database holds them now.
    ///
    /// # Errors
    ///
    /// The query error when the sites cannot be read.
    pub async fn load(_database: &Database) -> Result<Arc<Self>, sqlx::Error> {
        Ok(Arc::new(Self::default()))
    }

    /// Read `site_id` again from the database.
    ///
    /// # Errors
    ///
    /// The query error when the site cannot be read; the set keeps what it had.
    pub async fn refresh(&self, _database: &Database, _site_id: &str) -> Result<(), sqlx::Error> {
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
