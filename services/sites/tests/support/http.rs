//! Requests to the domain router, as Edge forwards them.

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use ratatoskr_sites::api::{ApiState, router};
use ratatoskr_sites::servable::Servable;
use ratatoskr_sites::sites_blobs::SitesBlobs;
use ratatoskr_sites::triggers::Shared;
use tower::ServiceExt as _;
use uuid::Uuid;

use super::{Failure, Fixture};

/// The largest answer body that a test reads.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// One answer of the router.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    /// The body as JSON.
    pub(crate) fn json(&self) -> Result<serde_json::Value, Failure> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// The error code of an error envelope, if the body is one.
    pub(crate) fn code(&self) -> Option<String> {
        self.json().ok()?.get("code")?.as_str().map(str::to_owned)
    }
}

/// A request with the anonymous claim of Edge's `public` class.
pub(crate) fn anonymous(method: Method, path: &str) -> Result<Request<Body>, Failure> {
    Ok(Request::builder()
        .method(method)
        .uri(path)
        .header("x-ratatoskr-session-kind", "anonymous")
        .header("x-correlation-id", "018f0000-0000-7000-8000-00000000c0c0")
        .body(Body::empty())?)
}

/// A request with the owner claims of Edge's `management` class.
pub(crate) fn owner(
    method: Method,
    path: &str,
    user: Uuid,
    body: Vec<u8>,
) -> Result<Request<Body>, Failure> {
    Ok(Request::builder()
        .method(method)
        .uri(path)
        .header("x-ratatoskr-user-id", user.to_string())
        .header("x-ratatoskr-session-kind", "browser")
        .header("x-correlation-id", "018f0000-0000-7000-8000-00000000c0c1")
        .body(Body::from(body))?)
}

/// Send `request` from `peer`.
pub(crate) async fn call_from(
    router: &Router,
    mut request: Request<Body>,
    peer: SocketAddr,
) -> Result<Answer, Failure> {
    request.extensions_mut().insert(ConnectInfo(peer));
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

/// Send `request` from a loopback peer, as Edge does.
pub(crate) async fn call(router: &Router, request: Request<Body>) -> Result<Answer, Failure> {
    call_from(router, request, SocketAddr::from(([127, 0, 0, 1], 40_000))).await
}

impl Fixture {
    /// The domain router over the database and the sites blob root of the fixture, with the state
    /// read now.
    pub(crate) async fn api(&self) -> Result<Router, Failure> {
        let shared = Shared {
            database: self.database.database.clone(),
            servable: Servable::load(&self.database.database).await?,
        };
        self.api_of(&shared).await
    }

    /// The domain router over `shared`, the state of a running service.
    pub(crate) async fn api_of(&self, shared: &Shared) -> Result<Router, Failure> {
        let state = ApiState::load(shared, SitesBlobs::new(self.sites_root.path())).await?;
        Ok(router(state))
    }
}

/// The four content routes of `site`, the same suffixes for every site.
pub(crate) fn content_routes(site: &str) -> [String; 4] {
    [
        format!("/v1/sites/{site}/content/release"),
        format!("/v1/sites/{site}/content/pages/home"),
        format!("/v1/sites/{site}/content/assets/{}", "0".repeat(64)),
        format!("/v1/sites/{site}/content/search?q=page"),
    ]
}

/// Assert that every content route of `site` answers exactly as for `unknown`, a site id that
/// never existed: the same status, the same headers apart from `date`, and the same body.
pub(crate) async fn assert_same_as_unknown(
    router: &Router,
    site: &str,
    unknown: &str,
) -> Result<(), Failure> {
    for (path, other) in content_routes(site).iter().zip(content_routes(unknown)) {
        let answer = call(router, anonymous(Method::GET, path)?).await?;
        let reference = call(router, anonymous(Method::GET, &other)?).await?;
        let mut headers = answer.headers.clone();
        let mut expected = reference.headers.clone();
        headers.remove("date");
        expected.remove("date");
        assert_eq!(answer.status, reference.status, "{path}: the status");
        assert_eq!(headers, expected, "{path}: the headers");
        assert_eq!(answer.body, reference.body, "{path}: the body");
    }
    Ok(())
}

/// Submit `manifest` for the fixture site as the owner `user`.
pub(crate) async fn submit(
    router: &Router,
    user: Uuid,
    manifest: &super::bus::TestManifest,
) -> Result<Answer, Failure> {
    call(
        router,
        owner(
            Method::PUT,
            &format!("/v1/sites/{}/manifest", super::SITE),
            user,
            manifest.bytes(),
        )?,
    )
    .await
}
