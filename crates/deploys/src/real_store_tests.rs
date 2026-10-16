//! Store acceptance with genuine SDK signatures. A wrapper injects delay without replacing proof verification.
use super::{BundleVerifier, real_proof_tests};
use crate::{DeployError, DeployStore, Limits};
use ratatoskr_sites_contracts::{DeployRequest, SiteId};
use ratatoskr_sites_persistence::test_support::TestDatabase;
use sha2::{Digest as _, Sha256};
use std::path::Path;

mod fixture {
    use super::*;
    pub(super) type Failure = Box<dyn std::error::Error>;
    pub(super) struct Store {
        pub(super) database: TestDatabase,
        pub(super) root: tempfile::TempDir,
        pub(super) site: SiteId,
        pub(super) request: DeployRequest,
        pub(super) deploys: DeployStore,
    }

    impl Store {
        pub(super) async fn new(
            verifier: BundleVerifier,
            request: DeployRequest,
        ) -> Result<Self, Failure> {
            let database = TestDatabase::create().await?;
            let root = tempfile::tempdir()?;
            let site = SiteId::parse("provenance-fixture")?;
            sqlx::query("insert into sites.sites (site_id, owner_id, created_at) values ($1, $2, '2000-01-01Z')")
                .bind(site.as_str()).bind(uuid::Uuid::nil()).execute(database.pool()).await?;
            let manifest = b"site_id = 'provenance-fixture'\nhostname = 'fixture.example.test'\npublications = ['bundle']\ncapabilities = ['pages.read']\n[github_repository]\nrepository = 'po4yka/ratatoskr-site-sdk'\nrepository_id = '1420131953'\n";
            let digest = crate::receipt::hex(&Sha256::digest(manifest));
            sqlx::query("insert into sites.site_manifests (site_id, digest, manifest, submitted_at, is_active) values ($1, $2, $3, '2000-01-01Z', true)")
                .bind(site.as_str()).bind(&digest).bind(manifest.as_slice()).execute(database.pool()).await?;
            sqlx::query("insert into sites.approval_projection (site_id, approved_digest, state, hostname, github_repository, changed_at) values ($1, $2, 'approved', 'fixture.example.test', $3, '2000-01-01Z')")
                .bind(site.as_str()).bind(&digest).bind(sqlx::types::Json(serde_json::json!({"repository":"po4yka/ratatoskr-site-sdk", "repository_id":"1420131953"})))
                .execute(database.pool()).await?;
            let deploys = DeployStore::new(
                database.database.clone(),
                root.path().to_path_buf(),
                Limits {
                    max_bundle_bytes: 64 * 1024 * 1024,
                    retention: 5,
                },
            )
            .with_verifier(verifier);
            Ok(Self {
                database,
                root,
                site,
                request,
                deploys,
            })
        }

        pub(super) async fn cleanup(self) -> Result<(), Failure> {
            self.deploys.shutdown().await;
            self.database.cleanup().await?;
            Ok(())
        }
    }

    pub(super) fn paused_verifier()
    -> Result<(tempfile::TempDir, BundleVerifier, DeployRequest), Failure> {
        use std::os::unix::fs::PermissionsExt as _;
        let (mut verifier, request, _) = real_proof_tests::verifier()?;
        let directory = tempfile::tempdir()?;
        let wrapper = directory.path().join("oras");
        let real = verifier
            .config
            .oras_path
            .as_os_str()
            .to_str()
            .ok_or("tool path is not UTF-8")?
            .replace('\'', "'\\''");
        let script = format!(
            "#!/bin/sh\nroot=$(/usr/bin/dirname \"$0\")\nif [ \"$1\" = manifest ]; then\n printf ready > \"$root/ready\"\n while [ -f \"$root/pause\" ]; do /bin/sleep 0.01; done\nfi\nexec '{real}' \"$@\"\n"
        );
        std::fs::write(&wrapper, script)?;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700))?;
        std::fs::write(directory.path().join("pause"), b"pause")?;
        verifier.config.oras_path = wrapper;
        Ok((directory, verifier, request))
    }

    pub(super) async fn ready(directory: &Path) -> Result<(), Failure> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !tokio::fs::try_exists(directory.join("ready"))
                .await
                .unwrap_or(false)
            {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }
}

use fixture::{Failure, Store};

#[tokio::test]
async fn a_real_build_is_stored_activated_and_retried() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    let first = store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    assert!(first.created);
    let target = store
        .deploys
        .site_dir(&store.site)
        .join("bundles")
        .join(store.request.commit.as_str());
    assert!(target.join("verification.json").is_file());
    let seal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(target.join("verification.json"))?)?;
    assert_eq!(
        seal.get("signer_workflow")
            .and_then(serde_json::Value::as_str),
        Some(crate::provenance::WORKFLOW)
    );
    assert_eq!(
        seal.get("signer_commit")
            .and_then(serde_json::Value::as_str),
        Some("89331e6aa0b845c3565c1d9785b0dd5038e2de81")
    );
    assert_eq!(
        seal.get("policy_version")
            .and_then(serde_json::Value::as_str),
        Some("site-bundle-proof-v1")
    );
    let activated = store
        .deploys
        .activate(&store.site, &store.request.commit)
        .await?;
    assert_eq!(activated.current_commit, store.request.commit);
    let retry = store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    assert!(!retry.created);
    store.cleanup().await
}

#[tokio::test]
async fn withdrawal_during_verify_stores_nothing() -> Result<(), Failure> {
    let (pause, verifier, request) = fixture::paused_verifier()?;
    let store = Store::new(verifier, request).await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let request = store.request.clone();
    let task = tokio::spawn(async move { deploys.receive_request(&site, request).await });
    fixture::ready(pause.path()).await?;
    sqlx::query("update sites.approval_projection set state = 'withdrawn', changed_at = '2001-01-01Z' where site_id = $1")
        .bind(store.site.as_str()).execute(store.database.pool()).await?;
    std::fs::remove_file(pause.path().join("pause"))?;
    assert!(task.await?.is_err());
    assert!(!store.deploys.site_dir(&store.site).join("bundles").exists());
    store.cleanup().await
}

#[tokio::test]
async fn capacity_is_bounded_and_cancel_cleans_staging() -> Result<(), Failure> {
    let (pause, verifier, request) = fixture::paused_verifier()?;
    let store = Store::new(verifier, request).await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let request = store.request.clone();
    let task = tokio::spawn(async move { deploys.receive_request(&site, request).await });
    fixture::ready(pause.path()).await?;
    assert!(matches!(
        store
            .deploys
            .receive_request(&store.site, store.request.clone())
            .await,
        Err(DeployError::Busy)
    ));
    task.abort();
    let _ = task.await;
    store.deploys.shutdown().await;
    assert_eq!(
        std::fs::read_dir(store.root.path().join(".verification"))?.count(),
        0
    );
    assert!(!store.deploys.site_dir(&store.site).join("bundles").exists());
    store.cleanup().await
}

#[tokio::test]
async fn verified_rollback_uses_stored_proof_without_network() -> Result<(), Failure> {
    let (verifier, first, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, first).await?;
    let second = real_proof_tests::second_request()?;
    for request in [&store.request, &second] {
        store
            .deploys
            .receive_request(&store.site, request.clone())
            .await?;
        store.deploys.activate(&store.site, &request.commit).await?;
    }
    // No tool may start during activation: remove both executable paths from the store policy.
    // The policy identity depends on versions, signer and trust root, not installation paths.
    let (mut verifier, _, _) = real_proof_tests::verifier()?;
    verifier.config.gh_path = "/does-not-exist/gh".into();
    verifier.config.oras_path = "/does-not-exist/oras".into();
    let offline = store.deploys.clone().with_verifier(verifier);
    let reverted = offline.activate(&store.site, &store.request.commit).await?;
    assert_eq!(reverted.current_commit, store.request.commit);
    assert_eq!(reverted.previous_commit.as_ref(), Some(&second.commit));
    store.cleanup().await
}

#[tokio::test]
async fn rebind_refuses_old_activation() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    let manifest = b"site_id = 'provenance-fixture'\nhostname = 'fixture.example.test'\npublications = ['bundle']\ncapabilities = ['pages.read']\n[github_repository]\nrepository = 'po4yka/ratatoskr-site-sdk'\nrepository_id = '999999'\n";
    let digest = crate::receipt::hex(&Sha256::digest(manifest));
    let mut tx = store.database.pool().begin().await?;
    sqlx::query("update sites.site_manifests set is_active = false where site_id = $1")
        .bind(store.site.as_str())
        .execute(&mut *tx)
        .await?;
    sqlx::query("insert into sites.site_manifests (site_id, digest, manifest, submitted_at, is_active) values ($1, $2, $3, '2001-01-01Z', true)").bind(store.site.as_str()).bind(&digest).bind(manifest.as_slice()).execute(&mut *tx).await?;
    sqlx::query("update sites.approval_projection set approved_digest = $2, github_repository = $3, changed_at = '2001-01-01Z' where site_id = $1")
        .bind(store.site.as_str()).bind(&digest).bind(sqlx::types::Json(serde_json::json!({"repository":"po4yka/ratatoskr-site-sdk", "repository_id":"999999"}))).execute(&mut *tx).await?;
    tx.commit().await?;
    assert!(matches!(
        store
            .deploys
            .activate(&store.site, &store.request.commit)
            .await,
        Err(DeployError::Provenance)
    ));
    assert!(!store.deploys.site_dir(&store.site).join("current").exists());
    store.cleanup().await
}

#[tokio::test]
async fn same_commit_other_digest_conflicts() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    let file = store
        .deploys
        .site_dir(&store.site)
        .join("bundles")
        .join(store.request.commit.as_str())
        .join("verification.json");
    // Fault injection models an existing record for a different artifact at this commit. The
    // incoming request still passes real cryptographic verification; no fake proof is accepted.
    let mut seal: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
    *seal.get_mut("artifact_sha256").ok_or("missing artifact")? = serde_json::json!("b".repeat(64));
    let before = serde_json::to_vec(&seal)?;
    std::fs::write(&file, &before)?;
    assert!(matches!(
        store
            .deploys
            .receive_request(&store.site, store.request.clone())
            .await,
        Err(DeployError::Conflict)
    ));
    assert_eq!(std::fs::read(&file)?, before);
    store.cleanup().await
}

#[tokio::test]
async fn concurrent_receipts_cannot_replace_proof() -> Result<(), Failure> {
    let (pause, verifier, request) = fixture::paused_verifier()?;
    let store = Store::new(verifier, request).await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let request = store.request.clone();
    let task = tokio::spawn(async move { deploys.receive_request(&site, request).await });
    fixture::ready(pause.path()).await?;
    assert!(matches!(
        store
            .deploys
            .receive_request(&store.site, store.request.clone())
            .await,
        Err(DeployError::Busy)
    ));
    std::fs::remove_file(pause.path().join("pause"))?;
    assert!(task.await??.created);
    let file = store
        .deploys
        .site_dir(&store.site)
        .join("bundles")
        .join(store.request.commit.as_str())
        .join("verification.json");
    let before = std::fs::read(&file)?;
    assert!(
        !store
            .deploys
            .receive_request(&store.site, store.request.clone())
            .await?
            .created
    );
    assert_eq!(std::fs::read(&file)?, before);
    store.cleanup().await
}

#[tokio::test]
async fn cancel_while_waiting_for_approval_lock_releases_the_job() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    let mut blocker = store.database.pool().begin().await?;
    sqlx::query("select site_id from sites.approval_projection where site_id = $1 for update")
        .bind(store.site.as_str())
        .execute(&mut *blocker)
        .await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let request = store.request.clone();
    let task = tokio::spawn(async move { deploys.receive_request(&site, request).await });
    waiting_for_approval_lock(&mut blocker).await?;
    let drained = tokio::time::timeout(std::time::Duration::from_secs(2), store.deploys.shutdown())
        .await
        .is_ok();
    // Always release the fixture lock before asserting, including the expected RED path.
    blocker.rollback().await?;
    let result = task.await?;
    let stored = store.deploys.site_dir(&store.site).join("bundles").exists();
    store.cleanup().await?;
    assert!(
        drained,
        "the cancelled verification still waited for the database lock"
    );
    assert!(result.is_err());
    assert!(!stored);
    Ok(())
}

async fn waiting_for_approval_lock(
    blocker: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), Failure> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            // Observe through the lock holder to keep the two-connection test pool bounded.
            // Refresh the transaction's cached activity snapshot before each observation.
            sqlx::query("select pg_stat_clear_snapshot()").execute(&mut **blocker).await?;
            let waiting_count: i64 = sqlx::query_scalar("select count(*) from pg_stat_activity where datname = current_database() and wait_event_type = 'Lock' and query like '%for update of p, m%'")
                .fetch_one(&mut **blocker).await?;
            if waiting_count > 0 { return Ok::<_, sqlx::Error>(()); }
            tokio::task::yield_now().await;
        }
    }).await??;
    Ok(())
}

#[tokio::test]
async fn shutdown_cancels_and_tracks_an_activation_waiting_for_approval() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    let mut blocker = store.database.pool().begin().await?;
    sqlx::query("select site_id from sites.approval_projection where site_id = $1 for update")
        .bind(store.site.as_str())
        .execute(&mut *blocker)
        .await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let commit = store.request.commit.clone();
    let mut task = tokio::spawn(async move { deploys.activate(&site, &commit).await });
    waiting_for_approval_lock(&mut blocker).await?;
    let second = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        store.deploys.activate(&store.site, &store.request.commit),
    )
    .await;
    let admission_bounded = matches!(second, Ok(Err(DeployError::Busy)));

    let drained = tokio::time::timeout(std::time::Duration::from_secs(2), store.deploys.shutdown())
        .await
        .is_ok();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(1), &mut task).await;
    let finished_at_shutdown = completed.is_ok();
    blocker.rollback().await?;
    let result = match completed {
        Ok(result) => result?,
        Err(_) => task.await?,
    };
    let current = store.deploys.site_dir(&store.site).join("current").exists();
    store.cleanup().await?;
    assert!(
        admission_bounded,
        "activation admission has an unbounded wait queue"
    );
    assert!(
        drained && finished_at_shutdown,
        "shutdown left an activation waiting outside its tracked jobs"
    );
    assert!(result.is_err());
    assert!(!current);
    Ok(())
}

#[tokio::test]
async fn an_unsealed_directory_is_refused_even_with_a_working_verifier() -> Result<(), Failure> {
    let (verifier, request, _) = real_proof_tests::verifier()?;
    let store = Store::new(verifier, request).await?;
    store
        .deploys
        .receive_request(&store.site, store.request.clone())
        .await?;
    let directory = store.deploys.site_dir(&store.site);
    std::fs::remove_file(
        directory
            .join("bundles")
            .join(store.request.commit.as_str())
            .join("verification.json"),
    )?;
    assert!(matches!(
        store
            .deploys
            .activate(&store.site, &store.request.commit)
            .await,
        Err(DeployError::Provenance)
    ));
    assert!(!directory.join("current").exists());
    store.cleanup().await
}

#[tokio::test]
async fn a_withdraw_and_reapprove_epoch_during_verification_is_refused() -> Result<(), Failure> {
    let (pause, verifier, request) = fixture::paused_verifier()?;
    let store = Store::new(verifier, request).await?;
    let deploys = store.deploys.clone();
    let site = store.site.clone();
    let request = store.request.clone();
    let job = tokio::spawn(async move { deploys.receive_request(&site, request).await });
    fixture::ready(pause.path()).await?;
    sqlx::query("update sites.approval_projection set state = 'withdrawn', changed_at = '2001-01-01Z' where site_id = $1").bind(store.site.as_str()).execute(store.database.pool()).await?;
    sqlx::query("update sites.approval_projection set state = 'approved', changed_at = '2002-01-01Z' where site_id = $1").bind(store.site.as_str()).execute(store.database.pool()).await?;
    std::fs::remove_file(pause.path().join("pause"))?;
    assert!(matches!(job.await?, Err(DeployError::Conflict)));
    assert!(!store.deploys.site_dir(&store.site).join("bundles").exists());
    store.cleanup().await
}
