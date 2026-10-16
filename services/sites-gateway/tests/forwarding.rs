//! SITE-026 tasks 7.1 and 7.2: the gateway forwards `/_r/v1/sites/<site>/...` of the site of the
//! host to Edge without the prefix, adds no identity, sets the client address, and streams
//! server-sent events at once (workspace spec `site-gateway`, design D7).

mod support;

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request};
use http_body_util::BodyExt as _;
use support::edge::StubEdge;
use support::{Failure, Tree, commit, router_of, send};

type Outcome = Result<(), Failure>;

/// The source address of the gateway in the tests. The host uses `127.0.0.83`; a developer machine
/// may have only `127.0.0.1` on its loopback interface.
const SOURCE: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A tree with `site-a` and `site-b`, and a gateway that forwards to `edge`.
fn gateway(edge: &StubEdge) -> Result<(Tree, axum::Router), Failure> {
    let tree = Tree::new()?;
    for (site, host) in [("site-a", "a.example"), ("site-b", "b.example")] {
        tree.site(site, host, true)?;
        tree.deploy(
            site,
            &commit('1'),
            &[("index.html", b"<title>x</title>", false)],
            true,
        )?;
        tree.activate(site, &commit('1'))?;
    }
    let mut config = tree.config();
    config.edge.url.clone_from(&edge.url);
    config.edge.source_address = SOURCE;
    let router = router_of(&ratatoskr_sites_gateway::Gateway::new(config));
    Ok((tree, router))
}

fn forwarded(path: &str) -> Result<Request<Body>, Failure> {
    Ok(Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("host", "a.example")
        .body(Body::empty())?)
}

#[tokio::test]
async fn another_sites_api_is_not_forwarded() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    for path in [
        "/_r/v1/sites/site-b/content/pages",
        "/_r/v1/sites/site-a/manifest",
        "/_r/v1/notes/connections",
    ] {
        let answer = send(&router, forwarded(path)?).await?;
        assert_eq!(answer.status, 404, "{path}");
        assert_eq!(
            answer.code().as_deref(),
            Some("sites.gateway.route_not_forwarded"),
            "{path}"
        );
    }
    assert_eq!(edge.seen().len(), 0, "the stub Edge receives no request");
    Ok(())
}

#[tokio::test]
async fn the_sites_own_api_is_forwarded_without_the_prefix() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let answer = send(&router, forwarded("/_r/v1/sites/site-a/content/pages")?).await?;
    let seen = edge.seen();
    assert_eq!(
        seen.iter()
            .map(|seen| (seen.method.as_str(), seen.path.as_str()))
            .collect::<Vec<_>>(),
        vec![("GET", "/v1/sites/site-a/content/pages")]
    );
    assert_eq!(answer.status, 201, "Edge's status");
    assert_eq!(
        answer.header("x-edge-answer"),
        Some("stub"),
        "Edge's headers"
    );
    assert_eq!(answer.body, br#"{"from":"edge"}"#, "Edge's body");
    Ok(())
}

#[tokio::test]
async fn inbound_ratatoskr_headers_are_removed() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut request = forwarded("/_r/v1/sites/site-a/content/pages")?;
    request
        .headers_mut()
        .insert("x-ratatoskr-user-id", "1".parse()?);
    request
        .headers_mut()
        .insert("x-ratatoskr-session-kind", "browser".parse()?);
    send(&router, request).await?;
    let seen = edge.seen();
    assert_eq!(seen.len(), 1, "the request is forwarded");
    assert!(
        seen.iter().all(|seen| !seen
            .headers
            .keys()
            .any(|name| name.as_str().starts_with("x-ratatoskr-user"))
            && !seen.headers.contains_key("x-ratatoskr-session-kind")),
        "no inbound x-ratatoskr- header reaches Edge"
    );
    Ok(())
}

#[tokio::test]
async fn client_ip_is_set_from_cf_connecting_ip() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut request = forwarded("/_r/v1/sites/site-a/content/pages")?;
    request
        .headers_mut()
        .insert("x-ratatoskr-client-ip", "1.1.1.1".parse()?);
    request
        .headers_mut()
        .insert("cf-connecting-ip", "2.2.2.2".parse()?);
    send(&router, request).await?;
    let seen = edge.seen();
    let values: Vec<&str> = seen
        .iter()
        .flat_map(|seen| seen.headers.get_all("x-ratatoskr-client-ip"))
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(
        values,
        vec!["2.2.2.2"],
        "exactly one header, from CF-Connecting-IP"
    );
    assert_eq!(
        seen.first().map(|seen| seen.peer.ip()),
        Some(SOURCE),
        "on a connection from the configured source address"
    );
    Ok(())
}

#[tokio::test]
async fn server_sent_events_arrive_within_100_ms_without_compression() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut request = forwarded("/_r/v1/sites/site-a/events")?;
    request
        .headers_mut()
        .insert("accept-encoding", "gzip, br".parse()?);
    let response = tower::ServiceExt::oneshot(router, request).await?;
    assert_eq!(edge.seen().len(), 1, "the stream is forwarded");
    assert_eq!(
        response.headers().get("content-encoding"),
        None,
        "no compression"
    );
    let mut body = response.into_body();
    let mut arrived = Vec::new();
    let mut text = String::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            arrived.push(std::time::Instant::now());
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    let sent = edge.sent();
    assert_eq!(sent.len(), 3);
    assert_eq!(arrived.len(), 3, "one frame per event: {text:?}");
    for (sent, arrived) in sent.iter().zip(&arrived) {
        assert!(
            arrived.duration_since(*sent) < Duration::from_millis(100),
            "an event arrives within 100 ms: {:?}",
            arrived.duration_since(*sent)
        );
    }
    assert!(
        text.find("data: 0") < text.find("data: 1") && text.find("data: 1") < text.find("data: 2"),
        "in the same order: {text:?}"
    );
    Ok(())
}

#[tokio::test]
async fn last_event_id_reaches_edge_unchanged() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut request = forwarded("/_r/v1/sites/site-a/events")?;
    request.headers_mut().insert("last-event-id", "42".parse()?);
    let response = tower::ServiceExt::oneshot(router, request).await?;
    drop(response);
    let seen = edge.seen();
    assert_eq!(
        seen.first()
            .and_then(|seen| seen.headers.get("last-event-id"))
            .and_then(|value| value.to_str().ok()),
        Some("42")
    );
    Ok(())
}

#[tokio::test]
async fn a_keep_alive_only_stream_stays_open_for_two_minutes() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let response =
        tower::ServiceExt::oneshot(router, forwarded("/_r/v1/sites/site-a/events?keepalive")?)
            .await?;
    assert_eq!(response.status(), 200, "the stream is open");
    // The clock is paused once the connection is open, so a paused clock never runs the connect
    // timeout of the gateway; the two minutes of the stream run on it.
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let mut body = response.into_body();
    let mut comments = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| format!("the stream broke: {error}"))?;
        if frame
            .into_data()
            .is_ok_and(|data| data.starts_with(b": keep-alive"))
        {
            comments += 1;
        }
    }
    assert_eq!(
        comments, 8,
        "every keep-alive comment of two minutes arrives"
    );
    assert!(
        started.elapsed() >= Duration::from_mins(2),
        "the connection stayed open for two minutes: {:?}",
        started.elapsed()
    );
    Ok(())
}

/// Review M1: a path with a dot segment, an empty segment, an encoded dot, slash or backslash, or a
/// backslash never reaches Edge, so it cannot climb out of the site's own routes.
#[tokio::test]
async fn dot_segments_and_encoded_separators_are_not_forwarded() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    for path in [
        "/_r/v1/sites/site-a/content/../../site-b/content/pages",
        "/_r/v1/sites/site-a/content/%2e%2e/deploys",
        "/_r/v1/sites/site-a/content/%2E%2E/%2E%2E/site-b/content/pages",
        "/_r/v1/sites/site-a/content/./pages",
        "/_r/v1/sites/site-a/content//pages",
        "/_r/v1/sites/site-a/content/a%2fb",
        "/_r/v1/sites/site-a/content/a%2Fb",
        "/_r/v1/sites/site-a/content/a%5cb",
        "/_r/v1/sites/site-a/content/a%5Cb",
        "/_r/v1/sites/site-a/content/a\\b",
    ] {
        let Ok(request) = forwarded(path) else {
            continue;
        };
        let answer = send(&router, request).await?;
        assert_eq!(
            answer.code().as_deref(),
            Some("sites.gateway.route_not_forwarded"),
            "{path}"
        );
        assert_eq!(answer.status, 404, "{path}");
    }
    assert_eq!(edge.seen().len(), 0, "the stub Edge receives no request");
    Ok(())
}

/// Review L5: an Edge response head has a finite thirty-second budget.
#[tokio::test]
async fn edge_that_sends_no_header_in_time_is_unavailable() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let answer = send(
        &router,
        forwarded("/_r/v1/sites/site-a/content/pages?slow-header")?,
    )
    .await?;
    assert_eq!(answer.status, 503);
    assert_eq!(answer.code().as_deref(), Some("sites.service_unavailable"));
    assert_eq!(edge.seen().len(), 1);
    Ok(())
}

/// Review L6: only public response headers reach the site client.
#[tokio::test]
async fn ratatoskr_headers_of_edge_are_removed() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let answer = send(&router, forwarded("/_r/v1/sites/site-a/content/pages")?).await?;
    assert_eq!(answer.status, 201);
    assert_eq!(answer.header("x-edge-answer"), Some("stub"));
    assert!(
        !answer
            .headers
            .keys()
            .any(|name| name.as_str().starts_with("x-ratatoskr-"))
    );
    Ok(())
}

/// Review L7: client-supplied forwarding metadata does not reach Edge.
#[tokio::test]
async fn forwarding_headers_of_the_client_are_removed() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut request = forwarded("/_r/v1/sites/site-a/content/pages")?;
    let removed = [
        "x-forwarded-for",
        "forwarded",
        "x-real-ip",
        "cf-ray",
        "cf-ipcountry",
    ];
    for name in removed {
        request.headers_mut().insert(name, "spoofed".parse()?);
    }
    request
        .headers_mut()
        .insert("cf-connecting-ip", "2.2.2.2".parse()?);
    let answer = send(&router, request).await?;
    assert_eq!(answer.status, 201);
    let seen = edge.seen();
    assert_eq!(seen.len(), 1);
    let headers = &seen[0].headers;
    for name in removed {
        assert!(!headers.contains_key(name), "{name}");
    }
    assert_eq!(
        headers
            .get("cf-connecting-ip")
            .and_then(|value| value.to_str().ok()),
        Some("2.2.2.2")
    );
    assert_eq!(
        headers
            .get("x-ratatoskr-client-ip")
            .and_then(|value| value.to_str().ok()),
        Some("2.2.2.2")
    );
    Ok(())
}
