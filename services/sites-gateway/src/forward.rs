//! Forwarding under `/_r/` to Edge (workspace spec `site-gateway`, design D7).
//!
//! Only `/_r/v1/sites/<site>/<route>` of the site that the host selects is forwarded, and only for a
//! route of [`ALLOWED`]. The gateway removes the `/_r` prefix, every inbound `x-ratatoskr-*` header
//! and the hop-by-hop fields, sets `X-Ratatoskr-Client-IP` from `CF-Connecting-IP`, and connects
//! from its configured loopback source address, the only peer whose client address Edge trusts. It
//! adds no identity: Edge authenticates the cookie or treats the request as anonymous. Both bodies
//! stream; an answer is passed on frame by frame, so a server-sent event reaches the client as soon
//! as Edge sends it, and it is never compressed.

use std::time::Duration;

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Uri, header};
use axum::response::{IntoResponse as _, Response};
use futures_util::StreamExt as _;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::Gateway;
use crate::config::EdgeConfig;
use crate::fault::Fault;

/// The prefix of the forwarded paths.
pub(crate) const PREFIX: &str = "/_r";

/// The routes below `/v1/sites/<site>/` that a site may reach from its own origin: the Content
/// Delivery API, the event stream, the sign-in, the release request, and the routes of the site
/// owner session (SITE-027). A route that ends with `/` holds every path below it. A change of the
/// list is a reviewed code change (design D7).
const ALLOWED: &[&str] = &[
    "content/",
    "events",
    "sessions",
    "releases",
    "drafts/",
    "edits",
    "conflicts/",
    "library",
    "knowledge/search",
];

/// How long a forwarded answer may send nothing: four times the keep-alive interval of Edge's
/// event streams (15 seconds).
const IDLE_READ: Duration = Duration::from_mins(1);

/// The header that names the client for Edge's rate limit.
const CLIENT_IP: &str = "x-ratatoskr-client-ip";

/// The address of the client that `cloudflared` sets.
const CF_CONNECTING_IP: &str = "cf-connecting-ip";

/// The client of Edge.
pub(crate) type EdgeClient = Client<HttpConnector, Body>;

/// A client that connects to Edge from `edge.source_address`.
pub(crate) fn client(edge: &EdgeConfig) -> EdgeClient {
    let mut connector = HttpConnector::new();
    connector.set_local_address(Some(edge.source_address));
    connector.set_connect_timeout(Some(Duration::from_secs(5)));
    Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(16)
        .build(connector)
}

/// Whether `path` is under the forwarding prefix.
pub(crate) fn is_forwarded(path: &str) -> bool {
    path == PREFIX || path.starts_with("/_r/")
}

/// Whether `below`, the path below `/v1/sites/<site>/`, is a route of [`ALLOWED`].
fn allowed(below: &str) -> bool {
    ALLOWED.iter().any(|route| {
        if route.ends_with('/') {
            below.len() > route.len() && below.starts_with(route)
        } else {
            below == *route
        }
    })
}

/// Forward the request of `parts` and `body` for `site` to Edge.
pub(crate) async fn forward(gateway: &Gateway, site: &str, parts: Parts, body: Body) -> Response {
    let path = parts.uri.path();
    let Some(edge_path) = path.strip_prefix(PREFIX) else {
        return Fault::NotForwarded.into_response();
    };
    let own = format!("/v1/sites/{site}/");
    if !edge_path.strip_prefix(&own).is_some_and(allowed) {
        return Fault::NotForwarded.into_response();
    }
    let query = parts
        .uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let Ok(uri) = format!("{}{edge_path}{query}", gateway.config.edge.url).parse::<Uri>() else {
        return Fault::NotForwarded.into_response();
    };
    let mut request = axum::http::Request::new(body);
    *request.method_mut() = parts.method.clone();
    *request.uri_mut() = uri;
    *request.headers_mut() = forwarded_headers(&parts.headers);
    let response = match gateway.edge.request(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "Edge could not be reached");
            return Fault::Unavailable.into_response();
        }
    };
    let (mut head, incoming) = response.into_parts();
    remove_hop_by_hop(&mut head.headers);
    let body = Body::from_stream(idle_bounded(Body::new(incoming)));
    Response::from_parts(head, body)
}

/// The data of `body`, frame by frame, ended with an error when no frame arrives within
/// [`IDLE_READ`].
fn idle_bounded(
    body: Body,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send {
    futures_util::stream::unfold(Some(body.into_data_stream()), |state| async move {
        let mut stream = state?;
        match tokio::time::timeout(IDLE_READ, stream.next()).await {
            Ok(Some(Ok(data))) => Some((Ok(data), Some(stream))),
            Ok(Some(Err(error))) => Some((Err(std::io::Error::other(error)), None)),
            Ok(None) => None,
            Err(_) => Some((
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Edge sent nothing within the idle-read budget",
                )),
                None,
            )),
        }
    })
}

/// The headers of a forwarded request: the client's, without its `x-ratatoskr-*` headers, its
/// `Host`, its `Accept-Encoding` and the hop-by-hop fields, and with `X-Ratatoskr-Client-IP` from
/// `CF-Connecting-IP`.
fn forwarded_headers(inbound: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in inbound {
        let reserved = name.as_str().starts_with("x-ratatoskr-");
        if reserved || *name == header::HOST || *name == header::ACCEPT_ENCODING {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    remove_hop_by_hop(&mut headers);
    if let Some(client) = inbound
        .get(CF_CONNECTING_IP)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<std::net::IpAddr>().ok())
        && let Ok(value) = HeaderValue::from_str(&client.to_string())
    {
        headers.insert(HeaderName::from_static(CLIENT_IP), value);
    }
    headers
}

/// Remove the hop-by-hop fields and the fields that `Connection` names.
fn remove_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
}
