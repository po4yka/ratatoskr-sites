//! A sites tree on a temporary directory, as the sites service writes it, and requests to the
//! router of the gateway.

#![allow(dead_code, reason = "each test file uses a part of the support")]

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use ratatoskr_sites_gateway::{Gateway, GatewayConfig, router};
use sha2::{Digest as _, Sha256};
use tower::ServiceExt as _;

pub(crate) type Failure = Box<dyn std::error::Error>;

/// The largest body that a test reads.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// A commit id of 40 hex digits that ends with `tail`.
pub(crate) fn commit(tail: char) -> String {
    format!("{}{tail}", "c".repeat(39))
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// One file of a deploy: its path, its bytes and its `hashed` flag.
pub(crate) type File<'a> = (&'a str, &'a [u8], bool);

/// A sites tree.
pub(crate) struct Tree {
    pub(crate) root: tempfile::TempDir,
}

impl Tree {
    pub(crate) fn new() -> Result<Self, Failure> {
        Ok(Self {
            root: tempfile::tempdir()?,
        })
    }

    pub(crate) fn site_dir(&self, site: &str) -> PathBuf {
        self.root.path().join(site)
    }

    /// Write `site.json` of `site`, by a temporary file and a rename as the sites service does.
    pub(crate) fn site(&self, site: &str, hostname: &str, approved: bool) -> Result<(), Failure> {
        let dir = self.site_dir(site);
        std::fs::create_dir_all(&dir)?;
        let hostnames: Vec<&str> = if approved { vec![hostname] } else { Vec::new() };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "site_id": site,
            "approved": approved,
            "hostnames": hostnames,
        }))?;
        let temporary = dir.join(".site.json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, dir.join("site.json"))?;
        Ok(())
    }

    /// Store a deploy of `site` at `commit` with `files`, the entry document `index.html` and the
    /// entry fallback `fallback`.
    pub(crate) fn deploy(
        &self,
        site: &str,
        commit: &str,
        files: &[File<'_>],
        fallback: bool,
    ) -> Result<(), Failure> {
        let dir = self.site_dir(site).join("bundles").join(commit);
        let mut listed: Vec<serde_json::Value> = Vec::new();
        let mut sorted = files.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (path, bytes, hashed) in &sorted {
            let target = dir.join("files").join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, bytes)?;
            listed.push(serde_json::json!({
                "path": path,
                "size_bytes": bytes.len(),
                "sha256": sha256(bytes),
                "hashed": hashed,
            }));
        }
        let manifest = serde_json::json!({
            "site_id": site,
            "repository": "owner/docs-site",
            "commit": commit,
            "entry_document": "index.html",
            "entry_fallback": fallback,
            "files": listed,
        });
        std::fs::write(
            dir.join("ratatoskr-bundle.json"),
            serde_json::to_vec(&manifest)?,
        )?;
        Ok(())
    }

    /// Point `current` of `site` at `commit`, and `previous` at the deploy that was current.
    pub(crate) fn activate(&self, site: &str, commit: &str) -> Result<(), Failure> {
        let dir = self.site_dir(site);
        let current = dir.join("current");
        if let Ok(before) = std::fs::read_link(&current) {
            let temporary = dir.join(".previous.tmp");
            std::os::unix::fs::symlink(before, &temporary)?;
            std::fs::rename(temporary, dir.join("previous"))?;
        }
        let temporary = dir.join(".current.tmp");
        std::os::unix::fs::symlink(PathBuf::from("bundles").join(commit), &temporary)?;
        std::fs::rename(temporary, current)?;
        Ok(())
    }

    /// A gateway over the tree.
    pub(crate) fn gateway(&self) -> Arc<Gateway> {
        Gateway::new(self.config())
    }

    /// The configuration of a gateway over the tree.
    pub(crate) fn config(&self) -> GatewayConfig {
        GatewayConfig {
            sites_root: self.root.path().to_path_buf(),
        }
    }
}

/// One answer of the router.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    /// The error code of an error envelope, if the body is one.
    pub(crate) fn code(&self) -> Option<String> {
        let body: serde_json::Value = serde_json::from_slice(&self.body).ok()?;
        body.get("code")?.as_str().map(str::to_owned)
    }

    /// The value of the header `name`.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }

    /// Whether the body holds `marker`.
    pub(crate) fn holds(&self, marker: &[u8]) -> bool {
        self.body
            .windows(marker.len())
            .any(|window| window == marker)
    }
}

/// Send `request` to `router`.
pub(crate) async fn send(router: &Router, request: Request<Body>) -> Result<Answer, Failure> {
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), MAX_BODY)
        .await?
        .to_vec();
    Ok(Answer {
        status,
        headers,
        body,
    })
}

/// `method path` with the `Host` header `host`.
pub(crate) fn request(method: Method, host: &str, path: &str) -> Result<Request<Body>, Failure> {
    Ok(Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .body(Body::empty())?)
}

/// `GET path` with the `Host` header `host`.
pub(crate) async fn get(router: &Router, host: &str, path: &str) -> Result<Answer, Failure> {
    send(router, request(Method::GET, host, path)?).await
}

/// The gateway router over `gateway`.
pub(crate) fn router_of(gateway: &Arc<Gateway>) -> Router {
    router(Arc::clone(gateway))
}
