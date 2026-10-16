//! SITE-026 tasks 4.9 to 4.12: the activation of a stored deploy and the retention of old deploys
//! (workspace spec `site-bundles`).

mod support;

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ratatoskr_sites_contracts::CommitSha;
use support::{Bundle, Failure, Store, commit};

type Outcome = Result<(), Failure>;

/// Store the bundle of `commit` for the site of `store`.
async fn deploy(store: &Store, commit: &str) -> Outcome {
    store
        .deploys
        .receive(
            &Store::site()?,
            std::io::Cursor::new(Bundle::new(commit).archive()?),
        )
        .await?;
    Ok(())
}

/// Activate `commit`, as the commit now current and the previous one, or the code of a refusal.
async fn activate(
    store: &Store,
    commit: &str,
) -> Result<Result<(String, Option<String>), String>, Failure> {
    Ok(store
        .deploys
        .activate(&Store::site()?, &CommitSha::parse(commit)?)
        .await
        .map(|activated| {
            (
                activated.current_commit.as_str().to_owned(),
                activated
                    .previous_commit
                    .map(|commit| commit.as_str().to_owned()),
            )
        })
        .map_err(|error| error.code().to_owned()))
}

/// The commit that the link `name` of the site points to.
fn link(store: &Store, name: &str) -> Result<Option<String>, Failure> {
    let path = store.deploys.site_dir(&Store::site()?).join(name);
    if std::fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    let resolved = std::fs::canonicalize(path)?;
    Ok(resolved
        .file_name()
        .map(|name| name.to_string_lossy().into_owned()))
}

/// Read the deploy that `current` resolves to once, and give back its commit when the directory is
/// complete: the manifest and the entry document name the same commit.
fn complete_deploy(current: &Path) -> Option<String> {
    let resolved: PathBuf = std::fs::canonicalize(current).ok()?;
    let manifest = std::fs::read(resolved.join("ratatoskr-bundle.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    let commit = manifest.get("commit")?.as_str()?.to_owned();
    let index = std::fs::read_to_string(resolved.join("files/index.html")).ok()?;
    index.contains(&commit).then_some(commit)
}

/// A reader that resolves `current` in a loop until `stop`, and counts each read that did not see a
/// complete deploy of one of `commits`.
struct Reader {
    current: PathBuf,
    stop: Arc<AtomicBool>,
    broken: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    commits: [String; 2],
}

impl Reader {
    fn run(&self) {
        while !self.stop.load(Ordering::SeqCst) {
            let seen = complete_deploy(&self.current);
            if !seen.is_some_and(|seen| self.commits.contains(&seen)) {
                self.broken.fetch_add(1, Ordering::SeqCst);
            }
            self.reads.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activation_switches_current_and_previous_atomically() -> Outcome {
    let store = Store::new(5).await?;
    let (a, b) = (commit('a'), commit('b'));
    deploy(&store, &a).await?;
    deploy(&store, &b).await?;
    assert_eq!(activate(&store, &a).await?, Ok((a.clone(), None)));

    let current = store.deploys.site_dir(&Store::site()?).join("current");
    let stop = Arc::new(AtomicBool::new(false));
    let broken = Arc::new(AtomicUsize::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let reader = Reader {
                current: current.clone(),
                stop: Arc::clone(&stop),
                broken: Arc::clone(&broken),
                reads: Arc::clone(&reads),
                commits: [a.clone(), b.clone()],
            };
            std::thread::spawn(move || reader.run())
        })
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let switched = activate(&store, &b).await?;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    stop.store(true, Ordering::SeqCst);
    for reader in readers {
        reader.join().map_err(|_| "a reader panicked")?;
    }

    assert_eq!(switched, Ok((b.clone(), Some(a.clone()))));
    assert_eq!(
        broken.load(Ordering::SeqCst),
        0,
        "every one of {} reads saw a complete deploy of a or of b",
        reads.load(Ordering::SeqCst)
    );
    assert_eq!(link(&store, "current")?, Some(b));
    assert_eq!(link(&store, "previous")?, Some(a));
    store.cleanup().await
}

#[tokio::test]
async fn activating_previous_needs_no_download() -> Outcome {
    let store = Store::new(5).await?;
    let (a, b) = (commit('a'), commit('b'));
    deploy(&store, &a).await?;
    deploy(&store, &b).await?;
    let stored_a = store
        .deploys
        .site_dir(&Store::site()?)
        .join("bundles")
        .join(&a);
    let inode = std::fs::metadata(&stored_a)?.ino();

    // The store has no HTTP client: a rollback reads nothing but the site directory, so the stored
    // deploy of `a` is served again as the same directory.
    assert_eq!(activate(&store, &a).await?, Ok((a.clone(), None)));
    assert_eq!(
        activate(&store, &b).await?,
        Ok((b.clone(), Some(a.clone())))
    );
    assert_eq!(
        activate(&store, &a).await?,
        Ok((a.clone(), Some(b.clone())))
    );
    assert_eq!(link(&store, "current")?, Some(a.clone()));
    assert_eq!(link(&store, "previous")?, Some(b));
    assert_eq!(
        std::fs::metadata(&stored_a)?.ino(),
        inode,
        "the deploy of a is the stored one, not a new copy"
    );
    store.cleanup().await
}

#[tokio::test]
async fn retention_keeps_current_and_previous() -> Outcome {
    let store = Store::new(5).await?;
    let commits: Vec<String> = ('1'..='6').map(commit).collect();
    let mut before: Option<String> = None;
    for each in &commits {
        // Each deploy is stored, and activated, later than the one before it.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        deploy(&store, each).await?;
        assert_eq!(
            activate(&store, each).await?,
            Ok((each.clone(), before.clone()))
        );
        before = Some(each.clone());
    }

    let mut kept: Vec<String> =
        std::fs::read_dir(store.deploys.site_dir(&Store::site()?).join("bundles"))?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
    kept.sort();
    let expected: Vec<String> = commits.iter().skip(1).cloned().collect();
    assert_eq!(
        kept, expected,
        "the oldest deploy that is neither current nor previous is removed, and five remain"
    );
    assert_eq!(link(&store, "current")?, Some(commit('6')));
    assert_eq!(link(&store, "previous")?, Some(commit('5')));
    store.cleanup().await
}

/// Review M4: a deploy that was stored after the last activation is waiting to be activated, so an
/// activation of another deploy never removes it, even with a retention of two.
#[tokio::test]
async fn retention_never_removes_a_deploy_stored_after_the_last_activation() -> Outcome {
    let store = Store::new(2).await?;
    let (a, b, c) = (commit('a'), commit('b'), commit('c'));
    deploy(&store, &a).await?;
    assert_eq!(activate(&store, &a).await?, Ok((a.clone(), None)));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    deploy(&store, &b).await?;
    assert_eq!(
        activate(&store, &b).await?,
        Ok((b.clone(), Some(a.clone())))
    );
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    // The owner uploads c ahead and rolls back to a before c is activated.
    deploy(&store, &c).await?;
    assert_eq!(
        activate(&store, &a).await?,
        Ok((a.clone(), Some(b.clone())))
    );
    let bundles = store.deploys.site_dir(&Store::site()?).join("bundles");
    assert!(
        bundles.join(&c).is_dir(),
        "the deploy stored after the last activation is kept"
    );
    assert_eq!(
        activate(&store, &c).await?,
        Ok((c, Some(a))),
        "and can be activated"
    );
    store.cleanup().await
}

/// The first activation has no earlier activation time: every other deploy is uploaded ahead.
#[tokio::test]
async fn the_first_activation_keeps_deploys_uploaded_ahead() -> Outcome {
    let store = Store::new(2).await?;
    for tail in ['a', 'b', 'c'] {
        deploy(&store, &commit(tail)).await?;
    }
    assert_eq!(
        activate(&store, &commit('a')).await?,
        Ok((commit('a'), None))
    );
    let bundles = store.deploys.site_dir(&Store::site()?).join("bundles");
    for tail in ['a', 'b', 'c'] {
        assert!(bundles.join(commit(tail)).is_dir(), "deploy {tail} remains");
    }
    store.cleanup().await
}
