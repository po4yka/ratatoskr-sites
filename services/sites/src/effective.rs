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

/// The effective set of `site_id`: the capabilities of the last approved manifest that the active
/// manifest also declares, the publications that both name, and the approved hostname. Without an
/// approval, or after a withdrawal, it is empty. An addition takes effect only after an approval; a
/// removal stops at once.
///
/// # Errors
///
/// The query error when the rows cannot be read.
pub async fn effective_set(database: &Database, site_id: &str) -> Result<Effective, sqlx::Error> {
    type Row = (
        String,
        Vec<String>,
        String,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select a.state, a.capabilities, a.hostname, active.manifest, approved.manifest \
         from sites.approval_projection a \
         left join sites.site_manifests active \
         on active.site_id = a.site_id and active.is_active \
         left join sites.site_manifests approved \
         on approved.site_id = a.site_id and approved.digest = a.approved_digest \
         where a.site_id = $1",
    )
    .bind(site_id)
    .fetch_optional(database.pool())
    .await?;
    let Some((state, approved_capabilities, hostname, Some(active), approved)) = row else {
        return Ok(Effective::default());
    };
    if state != "approved" {
        return Ok(Effective::default());
    }
    let Ok(active) = ratatoskr_sites_manifest::parse(&active) else {
        return Ok(Effective::default());
    };
    let capabilities = active
        .capabilities
        .into_iter()
        .filter(|capability| {
            approved_capabilities
                .iter()
                .any(|approved| approved == capability.as_str())
        })
        .collect();
    // The approval event names the digest; the publications come from the bytes that this service
    // stored under it. An approval of a digest that was never submitted here names none.
    let approved_publications: BTreeSet<String> = approved
        .and_then(|bytes| ratatoskr_sites_manifest::parse(&bytes).ok())
        .map(|manifest| {
            manifest
                .publications
                .iter()
                .map(|publication| publication.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let publications = active
        .publications
        .iter()
        .map(|publication| publication.as_str().to_owned())
        .filter(|publication| approved_publications.contains(publication))
        .collect();
    Ok(Effective {
        capabilities,
        publications,
        hostname: Some(hostname),
    })
}
