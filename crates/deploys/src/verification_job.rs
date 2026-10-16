//! Tracked deployment jobs retain the slot and cleanup after an HTTP caller disconnects.
use crate::{
    DeployError, DeployStore, authority, receipt, seal::Seal, verification_process::ProcessError,
};
use ratatoskr_sites_contracts::{DeployRequest, DeployStored, SiteId};
use std::path::{Path, PathBuf};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) struct PrivateDirectory(PathBuf);
impl PrivateDirectory {
    pub(super) fn create(root: &Path) -> Result<Self, DeployError> {
        use std::os::unix::fs::DirBuilderExt as _;
        let path = root
            .join(".verification")
            .join(uuid::Uuid::now_v7().simple().to_string());
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .map_err(DeployError::Storage)?;
        Ok(Self(path))
    }
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl From<ProcessError> for DeployError {
    fn from(error: ProcessError) -> Self {
        match error {
            ProcessError::Unavailable | ProcessError::Cancelled => Self::VerifierUnavailable,
            ProcessError::Rejected => Self::Provenance,
            ProcessError::Timeout => Self::Timeout,
            ProcessError::TooLarge => Self::TooLarge,
        }
    }
}

pub(super) fn check(cancel: &CancellationToken, deadline: Instant) -> Result<(), DeployError> {
    if cancel.is_cancelled() {
        return Err(DeployError::VerifierUnavailable);
    }
    if Instant::now() >= deadline {
        return Err(DeployError::Timeout);
    }
    Ok(())
}

/// Cancellation is safe here only while no filesystem publication has started. A caller that
/// selects over BEGIN must first mark its owned `SQLx` 0.8 connection close-on-drop.
pub(super) async fn bounded<F, T, E>(
    deadline: Instant,
    cancel: &CancellationToken,
    future: F,
) -> Result<T, DeployError>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: Into<DeployError>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(DeployError::VerifierUnavailable),
        () = tokio::time::sleep_until(deadline) => Err(DeployError::Timeout),
        result = future => result.map_err(Into::into),
    }
}

impl DeployStore {
    /// Verify one digest and proof against the current approval before any deploy is published.
    ///
    /// # Errors
    /// Returns a policy refusal, a busy slot, a deadline or unavailable verifier.
    pub async fn receive_request(
        &self,
        site: &SiteId,
        request: DeployRequest,
    ) -> Result<DeployStored, DeployError> {
        if self.stop.is_cancelled() {
            self.metrics
                .verifications
                .record::<()>(&Err(DeployError::VerifierUnavailable), true);
            return Err(DeployError::VerifierUnavailable);
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            self.metrics
                .verifications
                .record::<()>(&Err(DeployError::Busy), false);
            DeployError::Busy
        })?;
        let cancel = self.stop.child_token();
        let _caller = cancel.clone().drop_guard();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let store = self.clone();
        let site = site.clone();
        let deadline = Instant::now() + std::time::Duration::from_mins(2);
        self.jobs.spawn(async move {
            let _permit = permit;
            let answer = store
                .verify_and_prepare(&site, &request, deadline, &cancel)
                .await;
            store
                .metrics
                .verifications
                .record(&answer, cancel.is_cancelled());
            let outcome = answer.as_ref().map_or_else(DeployError::code, |_| "stored");
            tracing::info!(outcome, "site bundle verification ended");
            let _ = sender.send(answer);
        });
        receiver
            .await
            .map_err(|_| DeployError::VerifierUnavailable)?
    }

    /// Cancel verification and await process reaping and staging cleanup before runtime shutdown.
    pub async fn shutdown(&self) {
        self.stop.cancel();
        self.jobs.close();
        self.jobs.wait().await;
    }

    async fn verify_and_prepare(
        &self,
        site: &SiteId,
        request: &DeployRequest,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<DeployStored, DeployError> {
        let verifier = self
            .verifier
            .as_ref()
            .ok_or(DeployError::VerifierUnavailable)?;
        check(cancel, deadline)?;
        let approval = bounded(deadline, cancel, async {
            let mut connection = self.database.pool().acquire().await?;
            connection.close_on_drop();
            authority::read(&mut connection, site, false).await
        })
        .await?;
        let directory = PrivateDirectory::create(&self.root)?;
        let archive = verifier
            .verify(
                &approval.binding,
                request,
                directory.path(),
                self.limits
                    .max_bundle_bytes
                    .saturating_add(16 * 1024 * 1024),
                deadline,
                cancel,
            )
            .await?;
        let site_dir = self.site_dir(site);
        let owned_site = site.clone();
        let limit = self.limits.max_bundle_bytes;
        let owned_cancel = cancel.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&archive.archive).map_err(DeployError::Storage)?;
            let reader = CheckedReader {
                file,
                cancel: owned_cancel,
                deadline,
            };
            receipt::prepare(&site_dir, &owned_site, limit, reader)
        })
        .await
        .map_err(|_| DeployError::VerifierUnavailable)??;
        if prepared.manifest.commit != request.commit
            || !prepared
                .manifest
                .repository
                .as_str()
                .eq_ignore_ascii_case(approval.binding.repository.as_str())
        {
            return Err(DeployError::Provenance);
        }
        self.publish_verified(site, request, &approval, prepared, deadline, cancel)
            .await
    }

    async fn publish_verified(
        &self,
        site: &SiteId,
        request: &DeployRequest,
        approval: &authority::Approval,
        prepared: receipt::Prepared,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<DeployStored, DeployError> {
        use sqlx::Connection as _;
        let _serial = bounded(deadline, cancel, async {
            Ok::<_, DeployError>(self.activations.lock().await)
        })
        .await?;
        let mut connection = bounded(deadline, cancel, self.database.pool().acquire()).await?;
        // SQLx 0.8 BEGIN is not cancel-safe. Discard this connection on every path.
        connection.close_on_drop();
        let mut transaction = bounded(deadline, cancel, connection.begin()).await?;
        let current = bounded(
            deadline,
            cancel,
            authority::read(&mut transaction, site, true),
        )
        .await?;
        if &current != approval {
            return Err(DeployError::Conflict);
        }
        check(cancel, deadline)?;
        let verified_at: String = bounded(
            deadline,
            cancel,
            sqlx::query_scalar("select current_timestamp::text").fetch_one(&mut *transaction),
        )
        .await?;
        let verifier = self
            .verifier
            .as_ref()
            .ok_or(DeployError::VerifierUnavailable)?;
        let seal = Seal::new(approval, request, verifier, verified_at);
        let site_dir = self.site_dir(site);
        // Once rename starts, keep the row locks until the blocking task ends. Dropping this
        // future at a deadline could let an approval change precede a late publication.
        let owned_cancel = cancel.clone();
        let stored = tokio::task::spawn_blocking(move || {
            check(&owned_cancel, deadline)?;
            publish(&site_dir, prepared, &seal)
        })
        .await
        .map_err(|_| DeployError::VerifierUnavailable)??;
        // This transaction holds read locks only. Close it after rename, with no fallible
        // network operation that could report a completed filesystem publication as a failure.
        drop(transaction);
        crate::answer(stored)
    }
}

fn publish(
    site_dir: &Path,
    prepared: receipt::Prepared,
    seal: &Seal,
) -> Result<receipt::Stored, DeployError> {
    let target = site_dir
        .join(receipt::BUNDLES)
        .join(prepared.manifest.commit.as_str());
    if target.exists() {
        if !Seal::read(&target)?.same_artifact(seal) {
            return Err(DeployError::Conflict);
        }
        return receipt::stored(&target);
    }
    prepared.seal(&serde_json::to_vec(seal).map_err(|_| DeployError::Provenance)?)?;
    receipt::publish(site_dir, prepared)
}

struct CheckedReader {
    file: std::fs::File,
    cancel: CancellationToken,
    deadline: Instant,
}
impl std::io::Read for CheckedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        check(&self.cancel, self.deadline).map_err(std::io::Error::other)?;
        std::io::Read::read(&mut self.file, buffer)
    }
}
