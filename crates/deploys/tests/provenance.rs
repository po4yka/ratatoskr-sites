//! Legacy archive directories never supply evidence for activation.
mod support;
use ratatoskr_sites_contracts::CommitSha;
use support::{Bundle, Failure, Store, commit};

#[tokio::test]
async fn a_legacy_directory_cannot_be_activated() -> Result<(), Failure> {
    let store = Store::new(5).await?;
    let commit = commit('a');
    store
        .deploys
        .receive(
            &Store::site()?,
            std::io::Cursor::new(Bundle::new(&commit).archive()?),
        )
        .await?;
    let answer = store
        .deploys
        .activate(&Store::site()?, &CommitSha::parse(&commit)?)
        .await;
    assert!(
        answer.is_err(),
        "an archive without server verification was activated"
    );
    assert!(
        !store
            .deploys
            .site_dir(&Store::site()?)
            .join("current")
            .exists()
    );
    store.cleanup().await
}
