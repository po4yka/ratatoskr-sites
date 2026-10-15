//! SITE-026 tasks 6.5 and 6.6: the cache headers follow the manifest, and every static answer
//! carries the fixed security headers (workspace spec `site-gateway`, design D6).

mod support;

use support::{Failure, Tree, commit, get, router_of};

type Outcome = Result<(), Failure>;

/// The Content Security Policy of every static answer: scripts, styles, images, fonts and
/// connections from the site's own origin only, `data:` images, no plugin, no base change and no
/// framing.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
                   img-src 'self' data:; font-src 'self'; connect-src 'self'; \
                   object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

fn site() -> Result<Tree, Failure> {
    let tree = Tree::new()?;
    tree.site("site-a", "a.example", true)?;
    tree.deploy(
        "site-a",
        &commit('1'),
        &[
            ("index.html", b"<title>a</title>", false),
            ("assets/app.3f9a1c.js", b"console.log(1)", true),
            (
                ".ratatoskr/sign-in/index.html",
                b"<title>sign-in</title>",
                false,
            ),
        ],
        true,
    )?;
    tree.activate("site-a", &commit('1'))?;
    Ok(tree)
}

#[tokio::test]
async fn hashed_files_are_immutable_and_the_entry_document_is_short_lived() -> Outcome {
    let tree = site()?;
    let router = router_of(&tree.gateway());
    let hashed = get(&router, "a.example", "/assets/app.3f9a1c.js").await?;
    assert_eq!(
        hashed.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    let entry = get(&router, "a.example", "/").await?;
    assert_eq!(
        entry.header("cache-control"),
        Some("public, max-age=60, must-revalidate")
    );
    let fallback = get(&router, "a.example", "/notes/x").await?;
    assert_eq!(
        fallback.header("cache-control"),
        Some("public, max-age=60, must-revalidate"),
        "the entry document by the fallback"
    );
    Ok(())
}

#[tokio::test]
async fn every_static_response_carries_the_security_headers() -> Outcome {
    let tree = site()?;
    let router = router_of(&tree.gateway());
    for path in ["/", "/.ratatoskr/sign-in", "/assets/app.3f9a1c.js"] {
        let answer = get(&router, "a.example", path).await?;
        assert_eq!(answer.status, 200, "{path}");
        assert_eq!(
            answer.header("x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
        assert_eq!(
            answer.header("referrer-policy"),
            Some("strict-origin-when-cross-origin"),
            "{path}"
        );
        assert_eq!(answer.header("x-frame-options"), Some("DENY"), "{path}");
        assert_eq!(
            answer.header("content-security-policy"),
            Some(CSP),
            "{path}"
        );
    }
    Ok(())
}
