//! Static serving from the current deploy of a site (workspace spec `site-gateway`, design D5 and
//! D6).
//!
//! A request resolves `current` once and serves from that directory, so one answer never mixes two
//! deploys. Only a path that the manifest lists is served, so the manifest itself and any file
//! outside `files/` never are. A path with no file is looked up in `previous` when the previous
//! manifest marks it as hashed; then the entry document answers when the manifest enables the
//! fallback. Under `/.ratatoskr/` a folder path serves its `index.html` with `no-store`, and no
//! fallback applies.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use ratatoskr_sites_contracts::{BundleFile, BundleManifest};
use tokio_util::io::ReaderStream;

use crate::Gateway;
use crate::fault::Fault;

/// The manifest of a stored deploy.
const MANIFEST: &str = "ratatoskr-bundle.json";

/// The reserved path of the SDK pages.
const SDK_PREFIX: &str = ".ratatoskr";

/// The most manifests that the cache holds; it is emptied when it is full.
const MAX_CACHED_MANIFESTS: usize = 64;

/// The manifests of the deploys that requests resolved, by their directory. A stored deploy never
/// changes, so an entry is never stale.
#[derive(Debug, Default)]
pub(crate) struct Manifests {
    cached: Mutex<BTreeMap<PathBuf, Arc<BundleManifest>>>,
}

impl Manifests {
    async fn of(&self, dir: &Path) -> Option<Arc<BundleManifest>> {
        if let Some(found) = self.cached.lock().ok()?.get(dir) {
            return Some(Arc::clone(found));
        }
        let bytes = tokio::fs::read(dir.join(MANIFEST)).await.ok()?;
        let manifest: Arc<BundleManifest> = Arc::new(serde_json::from_slice(&bytes).ok()?);
        let mut cached = self.cached.lock().ok()?;
        if cached.len() >= MAX_CACHED_MANIFESTS {
            cached.clear();
        }
        cached.insert(dir.to_path_buf(), Arc::clone(&manifest));
        Some(manifest)
    }
}

/// A deploy, resolved once for one request.
struct Deploy {
    dir: PathBuf,
    manifest: Arc<BundleManifest>,
}

impl Deploy {
    async fn resolve(gateway: &Gateway, site_dir: &Path, name: &str) -> Option<Self> {
        let dir = tokio::fs::canonicalize(site_dir.join(name)).await.ok()?;
        let manifest = gateway.manifests.of(&dir).await?;
        Some(Self { dir, manifest })
    }

    /// The listed file of `path`: the path itself, or `index.html` of a folder path. `""` is the
    /// entry document.
    fn lookup(&self, path: &str) -> Option<&BundleFile> {
        if path.is_empty() {
            return self.file(self.manifest.entry_document.as_str());
        }
        self.file(path).or_else(|| {
            let folder = path.trim_end_matches('/');
            self.file(&format!("{folder}/index.html"))
        })
    }

    fn file(&self, path: &str) -> Option<&BundleFile> {
        let files = &self.manifest.files;
        let found = files
            .binary_search_by(|file| file.path.as_str().cmp(path))
            .ok()?;
        files.get(found)
    }
}

/// Serve the request from the current deploy of `site`.
pub(crate) async fn static_file(gateway: &Gateway, site: &str, parts: &Parts) -> Response {
    if parts.method != Method::GET && parts.method != Method::HEAD {
        return Fault::MethodNotAllowed.into_response();
    }
    let site_dir = gateway.config.sites.root.join(site);
    let Some(current) = Deploy::resolve(gateway, &site_dir, "current").await else {
        return Fault::NoDeploy.into_response();
    };
    let Ok(path) = percent_encoding::percent_decode_str(parts.uri.path()).decode_utf8() else {
        return Fault::PageNotFound.into_response();
    };
    let path = path.trim_start_matches('/');
    let reserved = path == SDK_PREFIX || path.starts_with(".ratatoskr/");
    if let Some(file) = current.lookup(path) {
        return answer(&current.dir, file, reserved, parts).await;
    }
    if reserved {
        return Fault::PageNotFound.into_response();
    }
    if let Some(previous) = Deploy::resolve(gateway, &site_dir, "previous").await
        && let Some(file) = previous.file(path).filter(|file| file.hashed)
    {
        return answer(&previous.dir, file, false, parts).await;
    }
    if current.manifest.entry_fallback
        && let Some(entry) = current.lookup("")
    {
        return answer(&current.dir, entry, false, parts).await;
    }
    Fault::PageNotFound.into_response()
}

/// The answer of `file` of the deploy in `dir`: 304 for a matching `If-None-Match`, else its bytes
/// streamed from disk (none for `HEAD`).
async fn answer(dir: &Path, file: &BundleFile, reserved: bool, parts: &Parts) -> Response {
    let etag = format!("\"{}\"", file.sha256.as_str());
    let mut response = if matches(parts, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let opened = tokio::fs::File::open(dir.join("files").join(file.path.as_str())).await;
        let Ok(opened) = opened else {
            tracing::warn!(
                path = file.path.as_str(),
                "a listed file of a deploy cannot be read"
            );
            return Fault::Unavailable.into_response();
        };
        let body = if parts.method == Method::HEAD {
            Body::empty()
        } else {
            Body::from_stream(ReaderStream::new(opened))
        };
        let mut response = (StatusCode::OK, body).into_response();
        if let Ok(length) = HeaderValue::from_str(&file.size_bytes.to_string()) {
            response
                .headers_mut()
                .insert(header::CONTENT_LENGTH, length);
        }
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(media_type(file.path.as_str())),
        );
        response
    };
    let headers = response.headers_mut();
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, etag);
    }
    let cache = if reserved {
        NO_STORE
    } else if file.hashed {
        IMMUTABLE
    } else {
        SHORT_LIVED
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    security_headers(headers);
    response
}

/// The `Cache-Control` of a file whose name carries a hash of its content.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The `Cache-Control` of every other static file, the entry document among them.
const SHORT_LIVED: &str = "public, max-age=60, must-revalidate";

/// The `Cache-Control` of the reserved SDK pages.
const NO_STORE: &str = "no-store";

/// The Content Security Policy of every static answer (workspace spec `site-gateway`): scripts,
/// styles, images, fonts and connections from the site's own origin only, `data:` images, no
/// plugin, no change of the base and no framing.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
     base-uri 'self'; frame-ancestors 'none'";

/// Set the fixed security headers of a static answer.
pub(crate) fn security_headers(headers: &mut axum::http::HeaderMap) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
}

/// Whether `If-None-Match` of the request names `etag`.
fn matches(parts: &Parts, etag: &str) -> bool {
    parts
        .headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == etag || tag == "*")
}

/// The media type of `path` by its extension.
fn media_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}
