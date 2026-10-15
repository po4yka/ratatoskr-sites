//! SITE-025 task 5.37 (review M1): a refresh of the servable sites that read the database before a
//! withdrawal does not write its stale answer after the refresh of the withdrawal.

mod support;

use std::sync::Arc;

use ratatoskr_sites::servable::Servable;
use ratatoskr_sites_contracts::SiteCapability;
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

#[tokio::test]
async fn withdrawal_is_not_overwritten_by_a_stale_refresh() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let database = fixture.database.database.clone();
    let servable = Servable::load(&database).await?;
    assert!(
        servable.serving(SITE, SiteCapability::PagesRead).is_some(),
        "the approved site serves"
    );

    // The refresh of the build worker reads the approved state and waits before it writes.
    let pause = servable.pause_next_refresh();
    let stale = {
        let servable = Arc::clone(&servable);
        let database = database.clone();
        tokio::spawn(async move { servable.refresh(&database, SITE).await })
    };
    pause.reached.notified().await;

    // The approval consumer commits the withdrawal and refreshes the site.
    sqlx::query("update sites.approval_projection set state = 'withdrawn'")
        .execute(fixture.database.pool())
        .await?;
    let withdrawal = {
        let servable = Arc::clone(&servable);
        let database = database.clone();
        tokio::spawn(async move { servable.refresh(&database, SITE).await })
    };
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    pause.release.notify_one();
    stale.await??;
    withdrawal.await??;
    assert_eq!(
        servable.serving(SITE, SiteCapability::PagesRead),
        None,
        "the withdrawn site serves nothing after both refreshes"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn delete_retry_refreshes_the_servable_sites() -> Outcome {
    use axum::http::{Method, StatusCode};
    use support::http::{anonymous, call, owner};

    let fixture = Fixture::new().await?;
    sqlx::query(
        "insert into sites.publications (site_id, publication_id, connection_id, mode, \
         included_folders, created_at) values ($1, 'pub-s', $2, 'all', '{private}', now())",
    )
    .bind(SITE)
    .bind(fixture.index.connection())
    .execute(fixture.database.pool())
    .await?;
    let both = support::bus::TestManifest::of(
        "r.example.test",
        &["pub-r", "pub-s"],
        &["pages.read", "assets.read", "search.publication"],
    );
    fixture.approve_directly(&both).await?;
    fixture.builder().build(&request(None)).await?;
    let api = fixture.api().await?;
    let page = format!("/v1/sites/{SITE}/content/pages/private/zq7plannote");
    assert_eq!(
        call(&api, anonymous(Method::GET, &page)?).await?.status,
        StatusCode::OK
    );

    // A first DELETE removed the row, and its refresh failed; the owner sends it again.
    sqlx::query("delete from sites.publications where publication_id = 'pub-s'")
        .execute(fixture.database.pool())
        .await?;
    let retry = call(
        &api,
        owner(
            Method::DELETE,
            &format!("/v1/sites/{SITE}/publications/pub-s"),
            fixture.owner,
            Vec::new(),
        )?,
    )
    .await?;
    assert_eq!(retry.status, StatusCode::NOT_FOUND);
    assert_eq!(
        call(&api, anonymous(Method::GET, &page)?).await?.status,
        StatusCode::NOT_FOUND,
        "the retry refreshes the servable sites, so the page stops"
    );
    fixture.cleanup().await
}
