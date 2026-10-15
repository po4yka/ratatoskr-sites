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
