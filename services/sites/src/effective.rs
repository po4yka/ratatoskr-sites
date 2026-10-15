//! The effective set of a site (workspace spec `site-manifest`, "The effective set of a site is
//! approved and active").

use std::collections::BTreeSet;

use ratatoskr_sites_contracts::SiteCapability;
use ratatoskr_sites_persistence::Database;

/// The capabilities, the publications and the hostname that are in force for a site.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effective {
    /// The capabilities in force.
    pub capabilities: BTreeSet<SiteCapability>,
    /// The publications in force.
    pub publications: BTreeSet<String>,
    /// The hostname in force, `None` when the set is empty.
    pub hostname: Option<String>,
}

/// The effective set of `site_id`: empty without an approval or after a withdrawal.
///
/// # Errors
///
/// The query error when the rows cannot be read.
pub async fn effective_set(database: &Database, site_id: &str) -> Result<Effective, sqlx::Error> {
    let row: Option<(String, Option<Vec<u8>>)> = sqlx::query_as(
        "select a.state, m.manifest from sites.approval_projection a \
         left join sites.site_manifests m on m.site_id = a.site_id and m.is_active \
         where a.site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(database.pool())
    .await?;
    let Some((state, Some(active))) = row else {
        return Ok(Effective::default());
    };
    if state != "approved" {
        return Ok(Effective::default());
    }
    let Ok(active) = ratatoskr_sites_manifest::parse(&active) else {
        return Ok(Effective::default());
    };
    Ok(Effective {
        capabilities: active.capabilities.into_iter().collect(),
        publications: active
            .publications
            .iter()
            .map(|publication| publication.as_str().to_owned())
            .collect(),
        hostname: Some(active.hostname.as_str().to_owned()),
    })
}
