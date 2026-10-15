//! Static serving from the current deploy of a site.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use ratatoskr_sites_contracts::BundleManifest;

use crate::Gateway;
use crate::fault::Fault;

/// The manifest of a stored deploy.
const MANIFEST: &str = "ratatoskr-bundle.json";

/// A deploy, resolved once for one request.
pub(crate) struct Deploy {
    /// The resolved directory `bundles/<commit>`.
    pub(crate) dir: PathBuf,
    /// Its manifest.
    pub(crate) manifest: BundleManifest,
}

impl Deploy {
    /// The deploy that the link `name` of `site_dir` names, if it resolves to a stored deploy.
    pub(crate) async fn resolve(site_dir: &Path, name: &str) -> Option<Self> {
        let dir = tokio::fs::canonicalize(site_dir.join(name)).await.ok()?;
        let bytes = tokio::fs::read(dir.join(MANIFEST)).await.ok()?;
        let manifest = serde_json::from_slice(&bytes).ok()?;
        Some(Self { dir, manifest })
    }
}

/// Serve the request from the current deploy of `site`.
pub(crate) async fn static_file(gateway: &Gateway, site: &str, parts: &Parts) -> Response {
    let _ = parts;
    let site_dir = gateway.config.sites.root.join(site);
    let Some(current) = Deploy::resolve(&site_dir, "current").await else {
        return Fault::NoDeploy.into_response();
    };
    let entry = current
        .dir
        .join("files")
        .join(current.manifest.entry_document.as_str());
    match tokio::fs::read(entry).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            Body::from(bytes),
        )
            .into_response(),
        Err(_) => Fault::Unavailable.into_response(),
    }
}
