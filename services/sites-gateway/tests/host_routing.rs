//! SITE-026 tasks 6.1 and 6.2: a hostname selects exactly one approved site with a current deploy
//! (workspace spec `site-gateway`).

mod support;

use std::time::{Duration, Instant};

use support::{Failure, Tree, commit, get, router_of};

type Outcome = Result<(), Failure>;

const MARK_A: &[u8] = b"zq7-bundle-of-site-a";
const MARK_B: &[u8] = b"zq7-bundle-of-site-b";

/// A tree with the approved sites `site-a` (`a.example`) and `site-b` (`b.example`), each with a
/// current deploy whose files carry its marker.
fn two_sites() -> Result<Tree, Failure> {
    let tree = Tree::new()?;
    for (site, host, mark) in [
        ("site-a", "a.example", MARK_A),
        ("site-b", "b.example", MARK_B),
    ] {
        tree.site(site, host, true)?;
        tree.deploy(
            site,
            &commit('1'),
            &[("index.html", mark, false), ("app.js", mark, true)],
            true,
        )?;
        tree.activate(site, &commit('1'))?;
    }
    Ok(tree)
}

#[tokio::test]
async fn an_unknown_host_gets_404_and_no_site_bytes() -> Outcome {
    let tree = two_sites()?;
    let router = router_of(&tree.gateway());
    for path in ["/", "/app.js", "/index.html", "/missing"] {
        let answer = get(&router, "unknown.example", path).await?;
        assert_eq!(answer.status, 404, "{path}");
        assert_eq!(
            answer.code().as_deref(),
            Some("sites.gateway.unknown_host"),
            "{path}"
        );
        assert!(
            !answer.holds(MARK_A) && !answer.holds(MARK_B),
            "{path}: no byte of either bundle"
        );
    }
    let known = get(&router, "A.Example:443", "/").await?;
    assert_eq!(
        known.status, 200,
        "a known host, in another case and with a port"
    );
    assert!(known.holds(MARK_A));
    Ok(())
}

#[tokio::test]
async fn an_approved_site_without_a_deploy_gets_no_deploy() -> Outcome {
    let tree = two_sites()?;
    tree.site("site-c", "c.example", true)?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "c.example", "/").await?;
    assert_eq!(answer.status, 404);
    assert_eq!(answer.code().as_deref(), Some("sites.gateway.no_deploy"));
    assert!(!answer.holds(MARK_A) && !answer.holds(MARK_B));
    Ok(())
}

#[tokio::test]
async fn a_withdrawn_site_is_dropped_within_five_seconds() -> Outcome {
    let tree = two_sites()?;
    let gateway = tree.gateway();
    let stop = tokio_util::sync::CancellationToken::new();
    let rescan = gateway.spawn_rescan(stop.clone());
    let router = router_of(&gateway);
    assert_eq!(
        get(&router, "a.example", "/").await?.status,
        200,
        "served first"
    );

    tree.site("site-a", "a.example", false)?;
    let rewritten = Instant::now();
    let mut late = Vec::new();
    while rewritten.elapsed() < Duration::from_secs(6) {
        let answer = get(&router, "a.example", "/").await?;
        if rewritten.elapsed() > Duration::from_secs(5) {
            late.push((answer.status.as_u16(), answer.code(), answer.holds(MARK_A)));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    stop.cancel();
    rescan.await?;
    assert!(!late.is_empty());
    assert!(
        late.iter().all(|(status, code, bytes)| *status == 404
            && code.as_deref() == Some("sites.gateway.unknown_host")
            && !bytes),
        "every request later than five seconds after the rewrite is 404 unknown_host: {late:?}"
    );
    Ok(())
}
