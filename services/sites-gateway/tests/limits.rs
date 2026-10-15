//! SITE-026 tasks 7.3 and 7.4: a token bucket per `CF-Connecting-IP` and a cap on forwarded bodies
//! (workspace spec `site-gateway`, design D8).

mod support;

use axum::body::Body;
use axum::http::{Method, Request};
use support::edge::StubEdge;
use support::{Failure, Tree, commit, router_of, send};

type Outcome = Result<(), Failure>;

const BURST: u32 = 5;

/// A gateway over one site with a burst of [`BURST`], one token a second, and a body cap of 16
/// bytes, forwarding to `edge`.
fn gateway(edge: &StubEdge) -> Result<(Tree, axum::Router), Failure> {
    let tree = Tree::new()?;
    tree.site("site-a", "a.example", true)?;
    tree.deploy(
        "site-a",
        &commit('1'),
        &[("index.html", b"<title>a</title>", false)],
        true,
    )?;
    tree.activate("site-a", &commit('1'))?;
    let mut config = tree.config();
    config.edge.url.clone_from(&edge.url);
    config.edge.source_address = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    config.limits.burst = BURST;
    config.limits.rate_per_second = 1;
    config.limits.forwarded_body_bytes = 16;
    let router = router_of(&ratatoskr_sites_gateway::Gateway::new(config));
    Ok((tree, router))
}

fn from(client: &str) -> Result<Request<Body>, Failure> {
    Ok(Request::builder()
        .method(Method::GET)
        .uri("/")
        .header("host", "a.example")
        .header("cf-connecting-ip", client)
        .body(Body::empty())?)
}

#[tokio::test]
async fn a_client_over_its_burst_gets_429_with_retry_after() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let mut answers = Vec::new();
    for _ in 0..BURST * 2 {
        let answer = send(&router, from("5.5.5.5")?).await?;
        answers.push((
            answer.status.as_u16(),
            answer.header("retry-after").is_some(),
        ));
    }
    let served = answers.iter().filter(|(status, _)| *status == 200).count();
    let limited: Vec<_> = answers
        .iter()
        .filter(|(status, _)| *status == 429)
        .collect();
    assert!(
        served <= usize::try_from(BURST)? + 1 && !limited.is_empty(),
        "the requests over the burst get 429: {answers:?}"
    );
    assert!(
        limited.iter().all(|(_, retry_after)| *retry_after),
        "each 429 carries Retry-After: {answers:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_second_client_is_not_affected() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    for _ in 0..BURST * 2 {
        send(&router, from("5.5.5.5")?).await?;
    }
    assert_eq!(
        send(&router, from("5.5.5.5")?).await?.status,
        429,
        "the first key is over its budget"
    );
    let other = send(&router, from("6.6.6.6")?).await?;
    assert_eq!(other.status, 200, "the second key is served normally");
    Ok(())
}

#[tokio::test]
async fn an_oversized_forwarded_body_gets_413_without_a_forward() -> Outcome {
    let edge = StubEdge::start().await?;
    let (_tree, router) = gateway(&edge)?;
    let body = vec![b'x'; 100];
    let request = Request::builder()
        .method(Method::POST)
        .uri("/_r/v1/sites/site-a/edits")
        .header("host", "a.example")
        .header("content-length", body.len())
        .body(Body::from(body))?;
    let answer = send(&router, request).await?;
    assert_eq!(answer.status, 413);
    assert_eq!(edge.seen().len(), 0, "the stub Edge receives no request");
    Ok(())
}
