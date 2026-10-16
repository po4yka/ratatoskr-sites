//! An activation owns the final approval lock until filesystem work ends, even after disconnect.
use crate::verification_job::{bounded, check};
use crate::{DeployError, DeployStore, activation, authority, receipt, seal};
use ratatoskr_sites_contracts::{CommitSha, DeployActivated, SiteId};
use sqlx::Connection as _;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

impl DeployStore {
    /// Activate a stored, verified commit under the current approval and policy. No tool is run.
    /// One activation can run at a time; the caller gets a busy error instead of an unbounded queue.
    ///
    /// # Errors
    /// Returns a missing site/deploy, invalid proof, busy slot or unavailable store.
    pub async fn activate(
        &self,
        site: &SiteId,
        commit: &CommitSha,
    ) -> Result<DeployActivated, DeployError> {
        let deadline = Instant::now() + std::time::Duration::from_mins(2);
        let admission = check(&self.stop, deadline);
        if admission.is_err() {
            self.metrics
                .activations
                .record(&admission, self.stop.is_cancelled());
        }
        admission?;
        let permit = self
            .activation_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                self.metrics
                    .activations
                    .record::<()>(&Err(DeployError::Busy), false);
                DeployError::Busy
            })?;
        let cancel = self.stop.child_token();
        let _caller = cancel.clone().drop_guard();
        let store = self.clone();
        let site = site.clone();
        let commit = commit.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.jobs.spawn(async move {
            let _permit = permit;
            let result = store
                .activate_verified(&site, &commit, deadline, &cancel)
                .await;
            store
                .metrics
                .activations
                .record(&result, cancel.is_cancelled());
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| DeployError::VerifierUnavailable)?
    }

    async fn activate_verified(
        &self,
        site: &SiteId,
        commit: &CommitSha,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<DeployActivated, DeployError> {
        bounded(deadline, cancel, self.require_approved(site)).await?;
        let verifier = self
            .verifier
            .as_ref()
            .ok_or(DeployError::VerifierUnavailable)?;
        let _serial = bounded(deadline, cancel, async {
            Ok::<_, DeployError>(self.activations.lock().await)
        })
        .await?;
        let mut connection = bounded(deadline, cancel, self.database.pool().acquire()).await?;
        // Keep cancellation during SQLx 0.8 BEGIN from returning an open transaction to the pool.
        connection.close_on_drop();
        let mut transaction = bounded(deadline, cancel, connection.begin()).await?;
        let approval = bounded(
            deadline,
            cancel,
            authority::read(&mut transaction, site, true),
        )
        .await?;
        let directory = self.site_dir(site);
        let owned_commit = commit.clone();
        let policy = verifier.policy_sha256();
        let signer = verifier.signer_commit().clone();
        let retention = self.limits.retention;
        let owned_cancel = cancel.clone();
        // The tracked worker awaits this task while it holds both locks. Cancelling the HTTP
        // caller cannot release authority ahead of a late filesystem switch.
        let previous = tokio::task::spawn_blocking(move || {
            check(&owned_cancel, deadline)?;
            let target = directory.join(receipt::BUNDLES).join(owned_commit.as_str());
            if !target.exists() {
                return Err(DeployError::DeployNotFound);
            }
            let proof = seal::Seal::read(&target)?;
            if !proof.matches(&approval, &owned_commit, &policy, &signer) {
                return Err(DeployError::Provenance);
            }
            check(&owned_cancel, deadline)?;
            activation::activate(&directory, owned_commit.as_str(), retention)
        })
        .await
        .map_err(|_| DeployError::VerifierUnavailable)??;
        // Only read locks were taken. Close after the filesystem switch without another
        // fallible network operation after publication.
        drop(transaction);
        let previous_commit = previous
            .as_deref()
            .map(CommitSha::parse)
            .transpose()
            .map_err(|_| DeployError::Storage(std::io::Error::other("a link names no commit")))?;
        Ok(DeployActivated {
            site_id: site.clone(),
            current_commit: commit.clone(),
            previous_commit,
        })
    }
}
