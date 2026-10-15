//! SITE-026 tasks 4.1 to 4.8: the receipt of a bundle archive (workspace spec `site-bundles`).

mod support;

use support::{Bundle, Failure, SITE, Store, append, append_pax_sized, append_raw, commit};

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

#[tokio::test]
async fn a_second_deploy_of_the_same_commit_returns_the_first() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('8'));
    let site = Store::site()?;
    let first = store
        .deploys
        .receive(&site, std::io::Cursor::new(bundle.archive()?))
        .await?;
    let second = store
        .deploys
        .receive(&site, std::io::Cursor::new(bundle.archive()?))
        .await
        .map_err(|error| error.code().to_owned());
    assert_eq!(
        second
            .as_ref()
            .map(|stored| (stored.commit.clone(), stored.created)),
        Ok((first.commit.clone(), false)),
        "the second call returns the stored deploy"
    );
    let bundles: Vec<String> = std::fs::read_dir(store.deploys.site_dir(&site).join("bundles"))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    assert_eq!(bundles, vec![commit('8')], "one directory for the commit");
    store.cleanup().await
}

/// Review M2: the limit counts the bytes of an entry, not the ustar size field, which a PAX `size`
/// record overrides.
#[tokio::test]
async fn a_pax_size_record_cannot_pass_the_limit() -> Outcome {
    let store = Store::with_limit(5, 4096).await?;
    let payload = vec![b'x'; 5000];
    let bundle = Bundle::new(&commit('9')).with("big.bin", &payload, false);
    let listed: Vec<_> = bundle
        .files
        .iter()
        .filter(|(path, ..)| path != "big.bin")
        .cloned()
        .collect();
    let archive = bundle.archive_with(&listed, |builder| {
        append_pax_sized(builder, "big.bin", &payload)
    })?;
    let refused = refusal(&store, archive).await?;
    assert_eq!(
        refused
            .as_ref()
            .map(|(code, status, _)| (code.as_str(), *status)),
        Some(("sites.bundle.too_large", 413)),
        "a bundle of 5000 bytes and more is over the limit of 4096: {refused:?}"
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

/// Review L1: a manifest that lists `ratatoskr-bundle.json` is refused as a reserved path.
#[tokio::test]
async fn a_manifest_that_lists_itself_is_reserved() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('d')).with("ratatoskr-bundle.json", b"{}", false);
    let listed: Vec<_> = bundle
        .files
        .iter()
        .filter(|(path, ..)| path != "ratatoskr-bundle.json")
        .cloned()
        .collect();
    let refused = refusal(&store, bundle.archive_with(&listed, |_| Ok(()))?).await?;
    assert_eq!(
        refused
            .as_ref()
            .map(|(code, status, _)| (code.as_str(), *status)),
        Some(("sites.bundle.reserved_path", 422))
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

/// Review L11: a file and a folder of one name are refused as unsafe paths, in either order.
#[tokio::test]
async fn a_file_and_a_folder_of_one_name_are_unsafe() -> Outcome {
    let store = Store::new(5).await?;
    for (first, second) in [("a", "a/b"), ("a/b", "a")] {
        let bundle = Bundle::new(&commit('e'))
            .with(first, b"first", false)
            .with(second, b"second", false);
        let mut ordered = bundle.files.clone();
        ordered.sort_by_key(|(path, ..)| usize::from(path != first));
        let refused = refusal(&store, bundle.archive_with(&ordered, |_| Ok(()))?).await?;
        assert_eq!(
            refused
                .as_ref()
                .map(|(code, status, _)| (code.as_str(), *status)),
            Some(("sites.bundle.unsafe_path", 422)),
            "{first} then {second}"
        );
        assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    }
    store.cleanup().await
}

/// Review L17: each kind of entry and path that the receipt refuses, with nothing stored.
#[tokio::test]
async fn links_devices_duplicates_and_non_nfc_paths_are_refused() -> Outcome {
    let store = Store::new(5).await?;
    let bundle = Bundle::new(&commit('f'));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "a hard link",
            bundle.archive_with(&bundle.files, |builder| {
                append_raw(
                    builder,
                    b"copy.html",
                    tar::EntryType::Link,
                    b"",
                    Some("index.html"),
                )
            })?,
        ),
        (
            "a device entry",
            bundle.archive_with(&bundle.files, |builder| {
                append_raw(builder, b"tty", tar::EntryType::Char, b"", None)
            })?,
        ),
        (
            "a duplicate entry",
            bundle.archive_with(&bundle.files, |builder| {
                append(builder, "index.html", b"<title>again</title>")
            })?,
        ),
        (
            "a path that is not NFC",
            bundle.archive_with(&bundle.files, |builder| {
                append(builder, "cafe\u{301}.html", b"<title>cafe</title>")
            })?,
        ),
    ];
    for (case, archive) in cases {
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
    store.cleanup().await
}

/// Review L17: a bundle whose files pass the limit is refused, with nothing stored.
#[tokio::test]
async fn a_bundle_over_the_limit_is_too_large() -> Outcome {
    let store = Store::with_limit(5, 64).await?;
    let bundle = Bundle::new(&commit('f')).with("big.bin", &[b'x'; 100], false);
    let refused = refusal(&store, bundle.archive()?).await?;
    assert_eq!(
        refused
            .as_ref()
            .map(|(code, status, _)| (code.as_str(), *status)),
        Some(("sites.bundle.too_large", 413))
    );
    assert_eq!(store.stored()?, Vec::<String>::new(), "nothing is stored");
    store.cleanup().await
}

/// Review L12: what a receipt left in `.staging/` when the process stopped is removed at the start.
#[tokio::test]
async fn stale_staging_is_removed_at_start() -> Outcome {
    let store = Store::new(5).await?;
    let stale = store
        .deploys
        .site_dir(&Store::site()?)
        .join(".staging")
        .join("0190c0de")
        .join("files");
    std::fs::create_dir_all(&stale)?;
    std::fs::write(stale.join("half.js"), b"half written")?;
    let removed = store.deploys.remove_stale_staging().await?;
    assert_eq!(
        store.stored()?,
        Vec::<String>::new(),
        "no stale file remains"
    );
    assert_eq!(removed, 1, "one stale receipt is removed");
    store.cleanup().await
}
