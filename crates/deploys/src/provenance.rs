//! Policy checks on output from a successful cryptographic verifier invocation.
//! The input of these checks is never the caller's unsigned predicate or proof object.

use serde_json::Value;

pub(super) const WORKFLOW: &str =
    "po4yka/ratatoskr-site-sdk/.github/workflows/reusable-site-bundle.yml";
pub(super) const ISSUER: &str = "https://token.actions.githubusercontent.com";
pub(super) const PREDICATE: &str = "https://slsa.dev/provenance/v1";

pub(super) struct Expected<'a> {
    pub(super) repository: &'a str,
    pub(super) repository_id: &'a str,
    pub(super) commit: &'a str,
    pub(super) artifact_sha256: &'a str,
    pub(super) signer_commit: &'a str,
}

pub(super) fn matches_identity(result: &Value, expected: &Expected<'_>) -> bool {
    let Some(verified) = result.get("verificationResult") else {
        return false;
    };
    let Some(certificate) = verified.pointer("/signature/certificate") else {
        return false;
    };
    let source = format!("https://github.com/{}", expected.repository);
    let signer = format!("https://github.com/{WORKFLOW}@{}", expected.signer_commit);
    let identity = certificate
        .get("sourceRepositoryURI")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(&source));
    let required = [
        ("issuer", ISSUER),
        ("sourceRepositoryIdentifier", expected.repository_id),
        ("sourceRepositoryDigest", expected.commit),
        ("buildSignerURI", signer.as_str()),
        ("subjectAlternativeName", signer.as_str()),
        ("buildSignerDigest", expected.signer_commit),
        ("runnerEnvironment", "github-hosted"),
        ("sourceRepositoryVisibilityAtSigning", "public"),
    ];
    if !identity
        || !required
            .iter()
            .all(|(key, wanted)| certificate.get(key).and_then(Value::as_str) == Some(*wanted))
    {
        return false;
    }
    let Some(statement) = verified.get("statement") else {
        return false;
    };
    let Some(subjects) = statement.get("subject").and_then(Value::as_array) else {
        return false;
    };
    statement.get("_type").and_then(Value::as_str) == Some("https://in-toto.io/Statement/v1")
        && statement.get("predicateType").and_then(Value::as_str) == Some(PREDICATE)
        && subjects.len() == 1
        && subjects.first().is_some_and(|subject| {
            subject.get("name").and_then(Value::as_str)
                == Some(format!("ghcr.io/{}", expected.repository).as_str())
                && subject.pointer("/digest/sha256").and_then(Value::as_str)
                    == Some(expected.artifact_sha256)
        })
}

#[cfg(test)]
mod tests {
    use super::{Expected, ISSUER, PREDICATE, WORKFLOW, matches_identity};
    use serde_json::{Value, json};

    const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SIGNER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn expected() -> Expected<'static> {
        Expected {
            repository: "owner/docs-site",
            repository_id: "12345",
            commit: COMMIT,
            artifact_sha256: DIGEST,
            signer_commit: SIGNER,
        }
    }

    // Synthetic output tests the parser only. Real signed evidence separately tests gh itself.
    fn verified_output() -> Value {
        json!({"verificationResult": {
            "signature": {"certificate": {
                "issuer": ISSUER, "sourceRepositoryURI": "https://github.com/owner/docs-site",
                "sourceRepositoryIdentifier": "12345", "sourceRepositoryDigest": COMMIT,
                "buildSignerURI": format!("https://github.com/{WORKFLOW}@{SIGNER}"),
                "subjectAlternativeName": format!("https://github.com/{WORKFLOW}@{SIGNER}"),
                "buildSignerDigest": SIGNER, "runnerEnvironment": "github-hosted",
                "sourceRepositoryVisibilityAtSigning": "public",
            }},
            "statement": {"_type": "https://in-toto.io/Statement/v1", "predicateType": PREDICATE,
                "subject": [{"name": "ghcr.io/owner/docs-site", "digest": {"sha256": DIGEST}}]},
        }})
    }

    #[test]
    fn only_matching_verified_certificate_passes() -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches_identity(&verified_output(), &expected()));
        for (field, changed) in [
            (
                "sourceRepositoryURI",
                "https://github.com/foreign/docs-site",
            ),
            ("sourceRepositoryIdentifier", "54321"),
            ("sourceRepositoryDigest", SIGNER),
            ("issuer", "https://untrusted.example"),
            (
                "buildSignerURI",
                "https://github.com/owner/foreign/.github/workflows/build.yml",
            ),
            (
                "subjectAlternativeName",
                "https://github.com/owner/foreign/.github/workflows/build.yml",
            ),
            ("buildSignerDigest", COMMIT),
            ("runnerEnvironment", "self-hosted"),
            ("sourceRepositoryVisibilityAtSigning", "private"),
        ] {
            let mut output = verified_output();
            output["verificationResult"]["signature"]["certificate"][field] = json!(changed);
            assert!(
                !matches_identity(&output, &expected()),
                "must refuse changed {field}"
            );
        }
        for pointer in [
            "/verificationResult/statement/subject/0/digest/sha256",
            "/verificationResult/statement/predicateType",
        ] {
            let mut output = verified_output();
            *output
                .pointer_mut(pointer)
                .ok_or("the fixture field is missing")? = json!("changed");
            assert!(
                !matches_identity(&output, &expected()),
                "must refuse changed {pointer}"
            );
        }
        assert!(
            !matches_identity(&json!({}), &expected()),
            "missing claims fail closed"
        );
        Ok(())
    }
}
