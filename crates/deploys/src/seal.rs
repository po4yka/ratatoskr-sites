//! Server-written proof metadata. Archive paths can never write this file.
use crate::{
    BundleVerifier, DeployError, authority::Approval, bundle_verifier::POLICY_VERSION,
    provenance::WORKFLOW,
};
use ratatoskr_sites_contracts::{CommitSha, DeployRequest, GitHubRepositoryBinding};
use std::path::Path;

pub(super) const FILE: &str = "verification.json";

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Seal {
    repository: GitHubRepositoryBinding,
    commit: CommitSha,
    artifact_sha256: String,
    policy_sha256: String,
    policy_version: String,
    signer_workflow: String,
    signer_commit: CommitSha,
    approval_sha256: String,
    verified_at: String,
}

impl Seal {
    pub(super) fn new(
        approval: &Approval,
        request: &DeployRequest,
        verifier: &BundleVerifier,
        verified_at: String,
    ) -> Self {
        Self {
            repository: approval.binding.clone(),
            commit: request.commit.clone(),
            artifact_sha256: request.artifact_sha256.as_str().to_owned(),
            policy_sha256: verifier.policy_sha256(),
            policy_version: POLICY_VERSION.to_owned(),
            signer_workflow: WORKFLOW.to_owned(),
            signer_commit: verifier.signer_commit().clone(),
            approval_sha256: approval.digest.clone(),
            verified_at,
        }
    }

    pub(super) fn matches(
        &self,
        approval: &Approval,
        commit: &CommitSha,
        policy: &str,
        signer: &CommitSha,
    ) -> bool {
        self.repository == approval.binding
            && &self.commit == commit
            && self.policy_sha256 == policy
            && self.policy_version == POLICY_VERSION
            && self.signer_workflow == WORKFLOW
            && &self.signer_commit == signer
    }

    pub(super) fn same_artifact(&self, other: &Self) -> bool {
        self.repository == other.repository
            && self.commit == other.commit
            && self.artifact_sha256 == other.artifact_sha256
            && self.policy_sha256 == other.policy_sha256
            && self.policy_version == other.policy_version
            && self.signer_workflow == other.signer_workflow
            && self.signer_commit == other.signer_commit
    }

    pub(super) fn read(path: &Path) -> Result<Self, DeployError> {
        use std::io::Read as _;
        let file = std::fs::File::open(path.join(FILE)).map_err(|_| DeployError::Provenance)?;
        let mut bytes = Vec::new();
        file.take(8193)
            .read_to_end(&mut bytes)
            .map_err(DeployError::Storage)?;
        if bytes.len() > 8192 {
            return Err(DeployError::Provenance);
        }
        serde_json::from_slice(&bytes).map_err(|_| DeployError::Provenance)
    }
}
