//! SITE-026 tasks 6.3 and 6.4: static paths serve the current deploy of the site of the host, the
//! hashed files of the previous deploy, and the reserved SDK path without fallback (workspace spec
//! `site-gateway`).

mod support;

use axum::http::Method;
use sha2::{Digest as _, Sha256};
use support::{Failure, Tree, commit, get, request, router_of, send};

type Outcome = Result<(), Failure>;

const ENTRY: &[u8] = b"<!doctype html><title>entry of a</title>";
const CALLBACK: &[u8] = b"<!doctype html><title>sign-in callback</title>";

fn quoted_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let hex = Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        });
    format!("\"{hex}\"")
}

/// `site-a` (`a.example`, fallback on) and `site-b` (`b.example`), each with an `app.js` of its own.
fn sites() -> Result<Tree, Failure> {
    let tree = Tree::new()?;
    tree.site("site-a", "a.example", true)?;
    tree.deploy(
        "site-a",
        &commit('1'),
        &[
            ("index.html", ENTRY, false),
            ("app.js", b"console.log('a')", false),
            (".ratatoskr/sign-in/index.html", CALLBACK, false),
        ],
        true,
    )?;
    tree.activate("site-a", &commit('1'))?;
    tree.site("site-b", "b.example", true)?;
    tree.deploy(
        "site-b",
        &commit('1'),
        &[
            ("index.html", b"<title>b</title>", false),
            ("app.js", b"console.log('b')", false),
        ],
        true,
    )?;
    tree.activate("site-b", &commit('1'))?;
    Ok(tree)
}

#[tokio::test]
async fn the_host_selects_the_file_of_its_own_site() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/app.js").await?;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, b"console.log('a')");
    let other = get(&router, "b.example", "/app.js").await?;
    assert_eq!(other.body, b"console.log('b')");
    Ok(())
}

#[tokio::test]
async fn a_matching_etag_returns_304() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let etag = quoted_sha256(b"console.log('a')");
    let first = get(&router, "a.example", "/app.js").await?;
    assert_eq!(
        first.header("etag"),
        Some(etag.as_str()),
        "the ETag is the SHA-256"
    );
    let mut conditional = request(Method::GET, "a.example", "/app.js")?;
    conditional
        .headers_mut()
        .insert("if-none-match", etag.parse()?);
    let answer = send(&router, conditional).await?;
    assert_eq!(answer.status, 304);
    assert!(answer.body.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_post_on_a_static_path_returns_405() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let answer = send(&router, request(Method::POST, "a.example", "/index.html")?).await?;
    assert_eq!(answer.status, 405);
    let stored = tree
        .site_dir("site-a")
        .join("bundles")
        .join(commit('1'))
        .join("files/index.html");
    assert_eq!(
        std::fs::read(stored)?,
        ENTRY,
        "the deploy on disk is unchanged"
    );
    Ok(())
}

#[tokio::test]
async fn the_entry_fallback_serves_the_entry_document() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/notes/some-page").await?;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, ENTRY);
    Ok(())
}

#[tokio::test]
async fn a_hashed_asset_of_the_previous_deploy_is_served() -> Outcome {
    let tree = sites()?;
    let old = b"console.log('old asset')";
    tree.deploy(
        "site-a",
        &commit('a'),
        &[
            ("index.html", ENTRY, false),
            ("assets/app.3f9a1c.js", old, true),
        ],
        true,
    )?;
    tree.activate("site-a", &commit('a'))?;
    tree.deploy(
        "site-a",
        &commit('b'),
        &[("index.html", b"<title>b</title>", false)],
        true,
    )?;
    tree.activate("site-a", &commit('b'))?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/assets/app.3f9a1c.js").await?;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, old);
    Ok(())
}

#[tokio::test]
async fn an_unhashed_file_of_the_previous_deploy_is_not_served() -> Outcome {
    let tree = sites()?;
    let about = b"<title>about of the previous deploy</title>";
    tree.deploy(
        "site-a",
        &commit('a'),
        &[("index.html", ENTRY, false), ("about.html", about, false)],
        true,
    )?;
    tree.activate("site-a", &commit('a'))?;
    tree.deploy(
        "site-a",
        &commit('b'),
        &[("index.html", b"<title>b</title>", false)],
        true,
    )?;
    tree.activate("site-a", &commit('b'))?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/about.html").await?;
    assert_ne!(answer.body, about, "never the previous file");
    assert!(
        answer.status == 404 || answer.body == b"<title>b</title>",
        "the entry fallback or 404: {} {:?}",
        answer.status,
        String::from_utf8_lossy(&answer.body)
    );
    Ok(())
}

#[tokio::test]
async fn the_sign_in_callback_is_served_with_no_store() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/.ratatoskr/sign-in?state=x").await?;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body, CALLBACK);
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    Ok(())
}

#[tokio::test]
async fn an_unknown_sdk_path_is_404_without_fallback() -> Outcome {
    let tree = sites()?;
    let router = router_of(&tree.gateway());
    let answer = get(&router, "a.example", "/.ratatoskr/other").await?;
    assert_eq!(answer.status, 404);
    assert_ne!(answer.body, ENTRY, "not the entry document");
    Ok(())
}
