//! The deploy routes that Edge forwards in the route class `deploy` (workspace change
//! `add-site-gateway-hosting`, design D3): `POST /v1/sites/{site}/deploys` with a digest and an untrusted proof
//! as its JSON body, and `POST /v1/sites/{site}/deploys/{commit}/activate`.
//!
//! Edge accepts the owner's primary session or a `site_deployer` device paired for the site. This
//! service checks the device claim again: a device that is not a `site_deployer` of this site is
//! refused before the body is read.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use ratatoskr_sites_contracts::{CommitSha, DeployActivated, DeployRequest, DeployStored, SiteId};
use ratatoskr_sites_deploys::DeployError;

use super::ApiState;
use super::fault::Fault;

/// The claim of the stored kind of the device of the session.
const DEVICE_KIND_HEADER: &str = "x-ratatoskr-device-kind";

/// The claim of the one site of a `site_deployer` device.
const SITE_ID_HEADER: &str = "x-ratatoskr-site-id";

/// The device kind that may deploy one site.
const SITE_DEPLOYER: &str = "site_deployer";

/// One MiB for the complete JSON request and its untrusted proof.
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

impl From<DeployError> for Fault {
    fn from(error: DeployError) -> Self {
        if matches!(error, DeployError::Storage(_) | DeployError::Database(_)) {
            tracing::warn!(%error, "a deploy failed");
        }
        Self::Deploy {
            status: StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_REQUEST),
            code: error.code(),
            message: error.to_string(),
        }
    }
}

/// Refuse a device that is not a `site_deployer` of `site`.
fn deployer_of(headers: &HeaderMap, site: &str) -> Result<(), Fault> {
    let Some(kind) = headers.get(DEVICE_KIND_HEADER) else {
        return Ok(());
    };
    let named = headers
        .get(SITE_ID_HEADER)
        .and_then(|value| value.to_str().ok());
    if kind.as_bytes() == SITE_DEPLOYER.as_bytes() && named == Some(site) {
        Ok(())
    } else {
        Err(Fault::Forbidden)
    }
}

/// `POST /v1/sites/{site}/deploys`: verify and store an immutable bundle.
pub(crate) async fn upload(
    State(state): State<Arc<ApiState>>,
    Path(site): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<DeployStored>, Fault> {
    deployer_of(&headers, &site)?;
    let site = SiteId::parse(&site).map_err(|_| Fault::SiteNotFound)?;
    if !headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(Fault::Deploy {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            code: "sites.request.unsupported_media_type",
            message: "the deploy request must use application/json".to_owned(),
        });
    }
    let bytes = axum::body::to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| Fault::from(DeployError::TooLarge))?;
    let request: DeployRequest = serde_json::from_slice(&bytes).map_err(|_| {
        Fault::from(DeployError::Invalid(
            "the deploy request is not valid".to_owned(),
        ))
    })?;
    if request.attestation_bundle.is_empty() {
        return Err(DeployError::Provenance.into());
    }
    Ok(Json(state.deploys.receive_request(&site, request).await?))
}

/// `POST /v1/sites/{site}/deploys/{commit}/activate`: make a stored deploy current.
pub(crate) async fn activate(
    State(state): State<Arc<ApiState>>,
    Path((site, commit)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<DeployActivated>, Fault> {
    deployer_of(&headers, &site)?;
    let site = SiteId::parse(&site).map_err(|_| Fault::SiteNotFound)?;
    let commit = CommitSha::parse(&commit).map_err(|_| Fault::from(DeployError::DeployNotFound))?;
    Ok(Json(state.deploys.activate(&site, &commit).await?))
}
