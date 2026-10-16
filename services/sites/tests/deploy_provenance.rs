//! SITE-028: deployment authority never substitutes for signed build provenance.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use support::http::call;
use support::{Failure, Fixture, SITE};

const COMMIT: &str = "3f6c2a9e8b1d4c7f0a5e2b9d6c3f8a1e4b7d0c2f";

fn archive() -> Result<Vec<u8>, Failure> {
    let page = b"<!doctype html><title>provenance</title>";
    let manifest = serde_json::to_vec(&serde_json::json!({
        "site_id": SITE, "repository": "owner/docs-site", "commit": COMMIT,
        "entry_document": "index.html", "entry_fallback": true,
        "files": [{ "path": "index.html", "size_bytes": page.len(),
            "sha256": ratatoskr_sites_release::sha256_hex(page), "hashed": false }],
    }))?;
    let mut tar = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        ("ratatoskr-bundle.json", manifest.as_slice()),
        ("index.html", page),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        tar.append_data(&mut header, name, bytes)?;
    }
    Ok(tar.into_inner()?)
}

fn request(
    owner: uuid::Uuid,
    device_site: Option<&str>,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<Request<Body>, Failure> {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/v1/sites/{SITE}/deploys"))
        .header("content-type", content_type)
        .header("x-ratatoskr-user-id", owner.to_string())
        .header("x-ratatoskr-session-kind", "browser")
        .header("x-correlation-id", "018f0000-0000-7000-8000-00000000c0c3");
    if let Some(site) = device_site {
        request = request
            .header("x-ratatoskr-session-kind", "device")
            .header("x-ratatoskr-device-id", uuid::Uuid::now_v7().to_string())
            .header("x-ratatoskr-device-kind", "site_deployer")
            .header("x-ratatoskr-site-id", site);
    }
    Ok(request.body(Body::from(bytes))?)
}

#[tokio::test]
async fn raw_tar_and_owner_cannot_bypass_provenance() -> Result<(), Failure> {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let foreign = call(
        &api,
        request(
            fixture.owner,
            Some("site-other"),
            "application/x-tar",
            archive()?,
        )?,
    )
    .await?;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
    for device in [None, Some(SITE)] {
        let answer = call(
            &api,
            request(fixture.owner, device, "application/x-tar", archive()?)?,
        )
        .await?;
        assert_eq!(
            answer.status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "raw tar must be refused before storage"
        );
        let unsigned = serde_json::to_vec(&serde_json::json!({
            "commit": COMMIT, "artifact_sha256": "a".repeat(64), "attestation_bundle": {},
        }))?;
        let answer = call(
            &api,
            request(fixture.owner, device, "application/json", unsigned)?,
        )
        .await?;
        assert_ne!(
            answer.status,
            StatusCode::OK,
            "an owner cannot bypass provenance"
        );
    }
    assert!(
        !fixture
            .deploys_root
            .path()
            .join(SITE)
            .join("bundles")
            .exists()
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn proof_size_and_foreign_transport_fields_are_refused() -> Result<(), Failure> {
    let fixture = Fixture::new().await?;
    let api = fixture.api().await?;
    let oversized = serde_json::to_vec(&serde_json::json!({
        "commit": COMMIT, "artifact_sha256": "a".repeat(64),
        "attestation_bundle": {"untrusted": "x".repeat(1024 * 1024)},
    }))?;
    let answer = call(
        &api,
        request(fixture.owner, None, "application/json", oversized)?,
    )
    .await?;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE);
    for field in ["registry", "archive_url", "trusted_root", "workflow", "tag"] {
        let mut body = serde_json::json!({
            "commit": COMMIT, "artifact_sha256": "a".repeat(64),
            "attestation_bundle": {"untrusted": true},
        });
        body[field] = serde_json::json!("https://127.0.0.1:1/never-fetch");
        let answer = call(
            &api,
            request(
                fixture.owner,
                None,
                "application/json",
                serde_json::to_vec(&body)?,
            )?,
        )
        .await?;
        assert_eq!(
            answer.status,
            StatusCode::BAD_REQUEST,
            "{field} was accepted"
        );
    }
    assert!(
        !fixture
            .deploys_root
            .path()
            .join(SITE)
            .join("bundles")
            .exists()
    );
    fixture.cleanup().await
}
