//! This lane runs real tools and an actual public SDK proof. Nothing is mocked or ignored.
use super::{BundleVerifier, VerifierConfig};
use ratatoskr_sites_contracts::{CommitSha, DeployRequest, GitHubRepositoryBinding};
use std::path::PathBuf;

type Failure = Box<dyn std::error::Error>;
const COMMIT: &str = "009afe32df0d5e10aaef79ae3e6bd6ecb3a989c9";
const SIGNER: &str = "89331e6aa0b845c3565c1d9785b0dd5038e2de81";
const DIGEST: &str = "d88bfae6f5ec3af637568587d6077674f1893b176a2015c111e083bad602b4a5";

#[expect(
    clippy::disallowed_methods,
    reason = "the opt-in test selects public fixtures and local tools, not service configuration"
)]
pub(super) fn verifier() -> Result<(BundleVerifier, DeployRequest, GitHubRepositoryBinding), Failure>
{
    let evidence = PathBuf::from(std::env::var("RATATOSKR_REAL_PROOF_DIR")?);
    let request = serde_json::from_value(serde_json::json!({
        "commit": COMMIT, "artifact_sha256": DIGEST,
        "attestation_bundle": serde_json::from_slice::<serde_json::Value>(&std::fs::read(evidence.join("proof.json"))?)?,
    }))?;
    let binding = serde_json::from_value(serde_json::json!({
        "repository": "po4yka/ratatoskr-site-sdk", "repository_id": "1420131953",
    }))?;
    let config = VerifierConfig {
        gh_path: PathBuf::from(std::env::var("RATATOSKR_REAL_GH")?),
        oras_path: PathBuf::from(std::env::var("RATATOSKR_REAL_ORAS")?),
        trusted_root_path: PathBuf::from(std::env::var("RATATOSKR_REAL_TRUST_ROOT")?),
        signer_commit: CommitSha::parse(SIGNER)?,
    };
    // This test constructor permits developer-owned tools. Cryptographic verification and all
    // production argv, environment, identity, digest and stream bounds remain the real path.
    let trusted_root = std::fs::read(&config.trusted_root_path)?;
    Ok((
        BundleVerifier {
            config,
            trusted_root,
        },
        request,
        binding,
    ))
}

#[tokio::test]
async fn a_real_approved_proof_yields_a_checked_archive() -> Result<(), Failure> {
    let (verifier, request, binding) = verifier()?;
    let directory = tempfile::tempdir()?;
    let archive = verifier
        .verify(
            &binding,
            &request,
            directory.path(),
            80 * 1024 * 1024,
            tokio::time::Instant::now() + std::time::Duration::from_mins(2),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await?;
    assert!(std::fs::metadata(archive.archive)?.len() > 0);
    Ok(())
}

fn corrupt_signature(request: &mut DeployRequest) -> Result<(), Failure> {
    let signature = request
        .attestation_bundle
        .get_mut("dsseEnvelope")
        .and_then(|envelope| envelope.pointer_mut("/signatures/0/sig"))
        .ok_or("missing real signature")?;
    let original = signature.as_str().ok_or("invalid real signature")?;
    let first = original.get(..1).ok_or("empty signature")?;
    let changed = format!(
        "{}{}",
        if first == "A" { "B" } else { "A" },
        original.get(1..).ok_or("invalid signature encoding")?
    );
    *signature = serde_json::Value::String(changed);
    Ok(())
}

#[tokio::test]
async fn a_real_proof_rejects_changed_identity_signature_source_and_signer() -> Result<(), Failure>
{
    for case in ["repository_id", "signature", "commit", "signer"] {
        let (mut verifier, mut request, mut binding) = verifier()?;
        match case {
            "repository_id" => {
                binding.repository_id =
                    ratatoskr_sites_contracts::GitHubRepositoryId::parse("999999")?;
            }
            "signature" => corrupt_signature(&mut request)?,
            "commit" => {
                request.commit = CommitSha::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
            }
            "signer" => {
                verifier.config.signer_commit =
                    CommitSha::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")?;
            }
            _ => return Err("unknown case".into()),
        }
        let directory = tempfile::tempdir()?;
        let result = verifier
            .verify(
                &binding,
                &request,
                directory.path(),
                80 * 1024 * 1024,
                tokio::time::Instant::now() + std::time::Duration::from_mins(2),
                &tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert!(result.is_err(), "{case} was accepted");
        assert!(
            !directory.path().join("bundle.tar").exists(),
            "{case} fetched an archive"
        );
    }
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the manual acceptance lane selects a second genuine public proof fixture"
)]
pub(super) fn second_request() -> Result<DeployRequest, Failure> {
    let evidence = PathBuf::from(std::env::var("RATATOSKR_REAL_SECOND_PROOF_DIR")?);
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence.join("summary.json"))?)?;
    let commit = summary
        .get("commit")
        .and_then(serde_json::Value::as_str)
        .ok_or("missing source commit")?;
    let digest = summary
        .get("artifact_digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or("missing artifact digest")?;
    Ok(serde_json::from_value(serde_json::json!({
        "commit": commit, "artifact_sha256": digest,
        "attestation_bundle": serde_json::from_slice::<serde_json::Value>(&std::fs::read(evidence.join("proof.json"))?)?,
    }))?)
}
