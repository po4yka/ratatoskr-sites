//! Short approval checks. Network calls and unpacking never hold a database lock.
use crate::DeployError;
use ratatoskr_sites_contracts::{GitHubRepositoryBinding, SiteId};
use sqlx::{PgConnection, Row as _};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Approval {
    pub(super) digest: String,
    pub(super) changed_at: String,
    pub(super) binding: GitHubRepositoryBinding,
}

pub(super) async fn read(
    connection: &mut PgConnection,
    site: &SiteId,
    lock: bool,
) -> Result<Approval, DeployError> {
    let query = if lock {
        "select p.approved_digest, p.changed_at::text as changed_at, p.github_repository \
         from sites.approval_projection p join sites.site_manifests m \
         on m.site_id = p.site_id and m.is_active and m.digest = p.approved_digest \
         where p.site_id = $1 and p.state = 'approved' for update of p, m"
    } else {
        "select p.approved_digest, p.changed_at::text as changed_at, p.github_repository \
         from sites.approval_projection p join sites.site_manifests m \
         on m.site_id = p.site_id and m.is_active and m.digest = p.approved_digest \
         where p.site_id = $1 and p.state = 'approved'"
    };
    let row = sqlx::query(query)
        .bind(site.as_str())
        .fetch_optional(connection)
        .await?
        .ok_or(DeployError::SiteNotFound)?;
    let binding: Option<sqlx::types::Json<GitHubRepositoryBinding>> =
        row.try_get("github_repository")?;
    Ok(Approval {
        digest: row.try_get("approved_digest")?,
        changed_at: row.try_get("changed_at")?,
        binding: binding.ok_or(DeployError::Provenance)?.0,
    })
}
