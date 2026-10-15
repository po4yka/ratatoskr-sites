//! SITE-026 tasks 4.1 to 4.8: the receipt of a bundle archive (workspace spec `site-bundles`).

mod support;

use support::{Bundle, Failure, SITE, Store, append, append_raw, commit};

type Outcome = Result<(), Failure>;

/// Receive `archive` for `SITE`, and the code, the status and the text of a refusal.
async fn refusal(
    store: &Store,
    archive: Vec<u8>,
) -> Result<Option<(String, u16, String)>, Failure> {
    let received = store
        .deploys
        .receive(&Store::site()?, std::io::Cursor::new(archive))
        .await;
    Ok(received
        .err()
        .map(|error| (error.code().to_owned(), error.status(), error.to_string())))
}

#[tokio::test]
async fn an_unlisted_file_is_refused() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('1'));
    let archive = bundle.archive_with(&bundle.files, |builder| {
        append(builder, "extra.js", b"alert(1)")
    })?;
    let refused = refusal(&store, archive).await?;
    assert_eq!(
        refused.as_ref().map(|(code, ..)| code.as_str()),
        Some("sites.bundle.unlisted_file")
    );
    assert!(
        refused
            .as_ref()
            .is_some_and(|(_, _, text)| text.contains("extra.js")),
        "the refusal names the file: {refused:?}"
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

#[tokio::test]
async fn a_digest_mismatch_names_the_path() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('2'));
    let mut changed = bundle.files.clone();
    if let Some((_, bytes, _)) = changed.iter_mut().find(|(path, ..)| path == "index.html")
        && let Some(byte) = bytes.first_mut()
    {
        *byte ^= 1;
    }
    let refused = refusal(&store, bundle.archive_with(&changed, |_| Ok(()))?).await?;
    assert_eq!(
        refused
            .as_ref()
            .map(|(code, status, _)| (code.as_str(), *status)),
        Some(("sites.bundle.digest_mismatch", 422))
    );
    assert!(
        refused
            .as_ref()
            .is_some_and(|(_, _, text)| text.contains("index.html")),
        "the refusal names the path: {refused:?}"
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

#[tokio::test]
async fn an_unsafe_path_writes_nothing() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('3'));
    let escape = bundle.archive_with(&bundle.files, |builder| {
        append_raw(
            builder,
            b"../../etc/passwd",
            tar::EntryType::Regular,
            b"root:x:0:0",
            None,
        )
    })?;
    let link = bundle.archive_with(&bundle.files, |builder| {
        append_raw(
            builder,
            b"passwd",
            tar::EntryType::Symlink,
            b"",
            Some("/etc/passwd"),
        )
    })?;
    for (case, archive) in [("a parent segment", escape), ("a symbolic link", link)] {
        let refused = refusal(&store, archive).await?;
        assert_eq!(
            refused.as_ref().map(|(code, ..)| code.as_str()),
            Some("sites.bundle.unsafe_path"),
            "{case}"
        );
        assert_eq!(
            store.stored()?,
            Vec::<String>::new(),
            "{case}: nothing is stored"
        );
    }
    assert!(
        !store.root.path().join("../etc/passwd").exists(),
        "no file is written outside the tree"
    );
    store.cleanup().await
}

#[tokio::test]
async fn a_site_mismatch_is_refused() -> Outcome {
    let store = Store::new(5).await?;
    let mut bundle = Bundle::new(&commit('4'));
    bundle.site = "site-b".to_owned();
    let refused = refusal(&store, bundle.archive()?).await?;
    assert_eq!(
        refused.as_ref().map(|(code, ..)| code.as_str()),
        Some("sites.bundle.site_mismatch")
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    assert_eq!(SITE, "site-a");
    store.cleanup().await
}

#[tokio::test]
async fn the_reserved_sdk_path_is_accepted() -> Outcome {
    let store = Store::new(5).await?;
    let callback = b"<!doctype html><title>sign-in</title>";
    let bundle = Bundle::new(&commit('5')).with(".ratatoskr/sign-in/index.html", callback, false);
    let stored = store
        .deploys
        .receive(&Store::site()?, std::io::Cursor::new(bundle.archive()?))
        .await?;
    assert!(stored.created);
    assert_eq!(stored.file_count, 3);
    let file = store
        .deploys
        .site_dir(&Store::site()?)
        .join("bundles")
        .join(commit('5'))
        .join("files/.ratatoskr/sign-in/index.html");
    assert_eq!(std::fs::read(file)?, callback, "the SDK page is stored");
    store.cleanup().await
}

#[tokio::test]
async fn a_path_under_the_forwarding_prefix_is_refused() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('6'));
    let archive = bundle.archive_with(&bundle.files, |builder| {
        append(builder, "_r/v1/sites/x/content", b"{}")
    })?;
    let refused = refusal(&store, archive).await?;
    assert_eq!(
        refused.as_ref().map(|(code, ..)| code.as_str()),
        Some("sites.bundle.reserved_path")
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

#[tokio::test]
async fn a_deploy_to_an_unapproved_site_is_refused() -> Outcome {
    let store = Store::new(5).await?;
    store.approval(SITE, "withdrawn").await?;
    let refused = refusal(&store, Bundle::new(&commit('7')).archive()?).await?;
    assert_eq!(
        refused
            .as_ref()
            .map(|(code, status, _)| (code.as_str(), *status)),
        Some(("sites.site_not_found", 404))
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}
