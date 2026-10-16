//! Verification of an immutable public OCI artifact before the archive receipt.

use sha2::{Digest as _, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::provenance::{Expected, ISSUER, PREDICATE, WORKFLOW, matches_identity};
use crate::verification_process::{OutputTarget, run};

use ratatoskr_sites_contracts::{CommitSha, DeployRequest, GitHubRepositoryBinding};

use crate::verification_process::ProcessError;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// Fixed server-owned verification policy. Request fields cannot replace these values.
pub struct VerifierConfig {
    /// Absolute path of the root-owned GitHub CLI 2.102.0.
    pub gh_path: PathBuf,
    /// Absolute path of the root-owned ORAS 1.3.0.
    pub oras_path: PathBuf,
    /// Absolute path of the reviewed server trust-root snapshot.
    pub trusted_root_path: PathBuf,
    /// Exact reviewed reusable-workflow revision.
    pub signer_commit: CommitSha,
}

#[derive(Debug)]
/// A verifier with checked executables and a private immutable trust-root snapshot.
pub struct BundleVerifier {
    config: VerifierConfig,
    trusted_root: Vec<u8>,
}

pub(crate) struct VerifiedArchive {
    pub(crate) archive: PathBuf,
}

pub(super) const POLICY_VERSION: &str = "site-bundle-proof-v1";

const MAX_METADATA: u64 = 1024 * 1024;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    media_type: String,
    artifact_type: String,
    #[serde(rename = "config")]
    _config: serde_json::Value,
    layers: Vec<Layer>,
    #[serde(default, rename = "annotations")]
    _annotations: std::collections::BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Layer {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default, rename = "annotations")]
    _annotations: std::collections::BTreeMap<String, String>,
}

impl BundleVerifier {
    /// Load root-owned tools and a bounded trust-root snapshot, then check exact tool versions.
    ///
    /// # Errors
    /// Returns an unavailable verifier when paths, ownership, versions or roots do not meet policy.
    pub async fn load(
        config: VerifierConfig,
        directory: &Path,
    ) -> Result<Self, crate::DeployError> {
        let checked = config.clone();
        let trusted_root = tokio::task::spawn_blocking(move || crate::preflight::root(&checked))
            .await
            .map_err(|_| crate::DeployError::VerifierUnavailable)?
            .map_err(|_| crate::DeployError::VerifierUnavailable)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let cancel = tokio_util::sync::CancellationToken::new();
        for (tool, argument, expected) in [
            (&config.gh_path, "--version", "gh version 2.102.0 "),
            (&config.oras_path, "version", "Version:    1.3.0"),
        ] {
            let output = run(
                tool,
                &[argument.into()],
                directory,
                deadline,
                &cancel,
                OutputTarget::Memory(4096),
            )
            .await
            .map_err(|_| crate::DeployError::VerifierUnavailable)?;
            let output = std::str::from_utf8(&output)
                .map_err(|_| crate::DeployError::VerifierUnavailable)?;
            let matches = if argument == "--version" {
                output
                    .lines()
                    .next()
                    .is_some_and(|line| line.starts_with(expected))
            } else {
                output.lines().any(|line| {
                    line.split_whitespace().collect::<Vec<_>>() == ["Version:", "1.3.0"]
                })
            };
            if !matches {
                return Err(crate::DeployError::VerifierUnavailable);
            }
        }
        Ok(Self {
            config,
            trusted_root,
        })
    }

    pub(crate) async fn verify(
        &self,
        binding: &GitHubRepositoryBinding,
        request: &DeployRequest,
        directory: &Path,
        max_archive_bytes: u64,
        deadline: tokio::time::Instant,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<VerifiedArchive, ProcessError> {
        self.private_files(request, directory).await?;
        let image = format!("ghcr.io/{}", binding.repository.as_str());
        let reference = format!("{image}@sha256:{}", request.artifact_sha256.as_str());
        let registry = directory.join("registry.json");
        let args = vec![
            OsString::from("manifest"),
            "fetch".into(),
            reference.into(),
            "--output".into(),
            "-".into(),
            "--registry-config".into(),
            registry.clone().into_os_string(),
        ];
        let manifest = run(
            &self.config.oras_path,
            &args,
            directory,
            deadline,
            cancel,
            OutputTarget::Memory(MAX_METADATA),
        )
        .await?;
        if hash(&manifest) != request.artifact_sha256.as_str() {
            return Err(ProcessError::Rejected);
        }
        let layer = checked_layer(&manifest, max_archive_bytes)?;
        let path = directory.join("manifest.json");
        write_new(&path, &manifest).await?;
        let args = self.verify_arguments(request, binding, directory);
        let output = run(
            &self.config.gh_path,
            &args,
            directory,
            deadline,
            cancel,
            OutputTarget::Memory(MAX_METADATA),
        )
        .await?;
        let results: Vec<serde_json::Value> =
            serde_json::from_slice(&output).map_err(|_| ProcessError::Rejected)?;
        let expected = Expected {
            repository: binding.repository.as_str(),
            repository_id: binding.repository_id.as_str(),
            commit: request.commit.as_str(),
            artifact_sha256: request.artifact_sha256.as_str(),
            signer_commit: self.config.signer_commit.as_str(),
        };
        if !results
            .iter()
            .any(|result| matches_identity(result, &expected))
        {
            return Err(ProcessError::Rejected);
        }
        let archive = directory.join("bundle.tar");
        let args = vec![
            OsString::from("blob"),
            "fetch".into(),
            format!("{image}@{}", layer.digest).into(),
            "--output".into(),
            "-".into(),
            "--registry-config".into(),
            registry.into_os_string(),
            "--no-tty".into(),
        ];
        run(
            &self.config.oras_path,
            &args,
            directory,
            deadline,
            cancel,
            OutputTarget::File(&archive, layer.size),
        )
        .await?;
        let owned = archive.clone();
        tokio::task::spawn_blocking(move || checked_archive(&owned, &layer))
            .await
            .map_err(|_| ProcessError::Unavailable)??;
        if cancel.is_cancelled() {
            return Err(ProcessError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ProcessError::Timeout);
        }
        Ok(VerifiedArchive { archive })
    }

    async fn private_files(
        &self,
        request: &DeployRequest,
        directory: &Path,
    ) -> Result<(), ProcessError> {
        let proof =
            serde_json::to_vec(&request.attestation_bundle).map_err(|_| ProcessError::Rejected)?;
        if proof.len() > usize::try_from(MAX_METADATA).unwrap_or(usize::MAX)
            || self.trusted_root.len() > usize::try_from(MAX_METADATA).unwrap_or(usize::MAX)
        {
            return Err(ProcessError::TooLarge);
        }
        if request.attestation_bundle.is_empty() || self.trusted_root.is_empty() {
            return Err(ProcessError::Rejected);
        }
        write_new(&directory.join("proof.json"), &proof).await?;
        write_new(&directory.join("trusted-root.jsonl"), &self.trusted_root).await?;
        write_new(&directory.join("registry.json"), b"{}\n").await
    }

    fn verify_arguments(
        &self,
        request: &DeployRequest,
        binding: &GitHubRepositoryBinding,
        directory: &Path,
    ) -> Vec<OsString> {
        vec![
            "attestation".into(),
            "verify".into(),
            directory.join("manifest.json").into_os_string(),
            "--bundle".into(),
            directory.join("proof.json").into_os_string(),
            "--custom-trusted-root".into(),
            directory.join("trusted-root.jsonl").into_os_string(),
            "--repo".into(),
            binding.repository.as_str().into(),
            "--source-digest".into(),
            request.commit.as_str().into(),
            "--signer-workflow".into(),
            WORKFLOW.into(),
            "--signer-digest".into(),
            self.config.signer_commit.as_str().into(),
            "--deny-self-hosted-runners".into(),
            "--cert-oidc-issuer".into(),
            ISSUER.into(),
            "--predicate-type".into(),
            PREDICATE.into(),
            "--digest-alg".into(),
            "sha256".into(),
            "--format".into(),
            "json".into(),
        ]
    }

    pub(crate) fn signer_commit(&self) -> &CommitSha {
        &self.config.signer_commit
    }

    pub(crate) fn policy_sha256(&self) -> String {
        hash(
            format!(
                "{POLICY_VERSION}\0gh=2.102.0\0oras=1.3.0\0{WORKFLOW}@{}\0{}",
                self.config.signer_commit.as_str(),
                hash(&self.trusted_root)
            )
            .as_bytes(),
        )
    }
}

fn hash(bytes: &[u8]) -> String {
    crate::receipt::hex(&Sha256::digest(bytes))
}

async fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ProcessError> {
    use tokio::io::AsyncWriteExt as _;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .await
        .map_err(|_| ProcessError::Unavailable)?;
    file.write_all(bytes)
        .await
        .map_err(|_| ProcessError::Unavailable)?;
    file.flush().await.map_err(|_| ProcessError::Unavailable)
}

fn checked_layer(bytes: &[u8], max_archive_bytes: u64) -> Result<Layer, ProcessError> {
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| ProcessError::Rejected)?;
    if manifest.schema_version != 2
        || manifest.media_type != "application/vnd.oci.image.manifest.v1+json"
        || manifest.artifact_type != "application/vnd.ratatoskr.site-bundle.v1"
        || manifest.layers.len() != 1
    {
        return Err(ProcessError::Rejected);
    }
    let layer = manifest
        .layers
        .into_iter()
        .next()
        .ok_or(ProcessError::Rejected)?;
    let digest = layer
        .digest
        .strip_prefix("sha256:")
        .ok_or(ProcessError::Rejected)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || layer.media_type != "application/vnd.ratatoskr.site-bundle.v1.tar"
        || layer.size == 0
    {
        return Err(ProcessError::Rejected);
    }
    if layer.size > max_archive_bytes {
        return Err(ProcessError::TooLarge);
    }
    Ok(layer)
}

fn checked_archive(path: &Path, layer: &Layer) -> Result<(), ProcessError> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|_| ProcessError::Unavailable)?;
    if file
        .metadata()
        .map_err(|_| ProcessError::Unavailable)?
        .len()
        != layer.size
    {
        return Err(ProcessError::Rejected);
    }
    let mut file = file.take(layer.size.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| ProcessError::Unavailable)?;
        if read == 0 {
            break;
        }
        let bytes = buffer.get(..read).ok_or(ProcessError::Unavailable)?;
        hasher.update(bytes);
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    if total != layer.size
        || layer.digest != format!("sha256:{}", crate::receipt::hex(&hasher.finalize()))
    {
        return Err(ProcessError::Rejected);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BundleVerifier, VerifierConfig};
    use ratatoskr_sites_contracts::{CommitSha, DeployRequest, GitHubRepositoryBinding};
    use sha2::{Digest as _, Sha256};
    use std::os::unix::fs::PermissionsExt as _;
    use std::time::Duration;

    type Failure = Box<dyn std::error::Error>;
    const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SIGNER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const WORKFLOW: &str = "po4yka/ratatoskr-site-sdk/.github/workflows/reusable-site-bundle.yml";

    #[tokio::test]
    async fn verified_manifest_is_checked_before_fetching_one_archive() -> Result<(), Failure> {
        // Fakes exercise orchestration only. Real signed site-bundle evidence remains required.
        let fixture = tempfile::tempdir()?;
        let directory = tempfile::tempdir()?;
        let archive = b"archive receipt bytes checked by its separate boundary tests";
        let layer_digest = crate::receipt::hex(&Sha256::digest(archive));
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": "application/vnd.ratatoskr.site-bundle.v1", "config": {},
            "layers": [{"mediaType": "application/vnd.ratatoskr.site-bundle.v1.tar",
                "digest": format!("sha256:{layer_digest}"), "size": archive.len()}],
        }))?;
        let digest = crate::receipt::hex(&Sha256::digest(&manifest));
        std::fs::write(fixture.path().join("manifest.json"), manifest)?;
        std::fs::write(fixture.path().join("archive.tar"), archive)?;
        let identity = serde_json::json!([{"verificationResult": {
            "signature": {"certificate": {
                "issuer": "https://token.actions.githubusercontent.com",
                "sourceRepositoryURI": "https://github.com/owner/docs-site",
                "sourceRepositoryIdentifier": "12345", "sourceRepositoryDigest": COMMIT,
                "buildSignerURI": format!("https://github.com/{WORKFLOW}@{SIGNER}"),
                "subjectAlternativeName": format!("https://github.com/{WORKFLOW}@{SIGNER}"),
                "buildSignerDigest": SIGNER, "runnerEnvironment": "github-hosted",
                "sourceRepositoryVisibilityAtSigning": "public",
            }}, "statement": {"_type": "https://in-toto.io/Statement/v1",
                "predicateType": "https://slsa.dev/provenance/v1", "subject": [{
                "name": "ghcr.io/owner/docs-site", "digest": {"sha256": digest}}]},
        }}]);
        std::fs::write(
            fixture.path().join("verification.json"),
            serde_json::to_vec(&identity)?,
        )?;
        let gh = fixture.path().join("gh");
        let oras = fixture.path().join("oras");
        std::fs::write(
            &gh,
            b"#!/bin/sh\nroot=$(/usr/bin/dirname \"$0\")\n/bin/cat \"$root/verification.json\"\n",
        )?;
        std::fs::write(&oras, b"#!/bin/sh\nroot=$(/usr/bin/dirname \"$0\")\nprintf '%s\\n' \"$*\" >> \"$root/calls\"\ncase \"$1 $2\" in\n'manifest fetch') /bin/cat \"$root/manifest.json\";;\n'blob fetch') /bin/cat \"$root/archive.tar\";;\n*) exit 1;;\nesac\n")?;
        for tool in [&gh, &oras] {
            std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o700))?;
        }
        let verifier = BundleVerifier {
            config: VerifierConfig {
                gh_path: gh,
                oras_path: oras,
                trusted_root_path: fixture.path().join("root"),
                signer_commit: CommitSha::parse(SIGNER)?,
            },
            trusted_root: b"{}".to_vec(),
        };
        let binding: GitHubRepositoryBinding = serde_json::from_value(serde_json::json!({
            "repository": "owner/docs-site", "repository_id": "12345"}))?;
        let request: DeployRequest = serde_json::from_value(serde_json::json!({
            "commit": COMMIT, "artifact_sha256": digest,
            "attestation_bundle": {"fixture": "not a real signature"},
        }))?;
        let result = verifier
            .verify(
                &binding,
                &request,
                directory.path(),
                4096,
                tokio::time::Instant::now() + Duration::from_secs(10),
                &tokio_util::sync::CancellationToken::new(),
            )
            .await?;
        assert_eq!(std::fs::read(result.archive)?, archive);
        assert_eq!(verifier.policy_sha256().len(), 64);
        let calls = std::fs::read_to_string(fixture.path().join("calls"))?;
        assert!(calls.contains(&format!("ghcr.io/owner/docs-site@sha256:{digest}")));
        assert!(calls.contains(&format!("ghcr.io/owner/docs-site@sha256:{layer_digest}")));
        assert!(!calls.contains("http://"));
        let manifest_call = calls
            .lines()
            .find(|line| line.starts_with("manifest fetch"))
            .ok_or("missing manifest call")?;
        assert!(
            !manifest_call.contains("--no-tty"),
            "ORAS 1.3.0 manifest fetch has no --no-tty flag"
        );
        Ok(())
    }
}

#[cfg(all(test, feature = "real-proof-tests"))]
#[path = "real_proof_tests.rs"]
mod real_proof_tests;

#[cfg(all(test, feature = "real-proof-tests"))]
#[path = "real_store_tests.rs"]
mod real_store_tests;
