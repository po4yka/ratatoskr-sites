//! SITE-025 task 5.21, review L1 (workspace 5.21e): the outbox relay publishes each row once, on its
//! subject, with the event id as `Nats-Msg-Id`, and marks it published after the `PubAck`. It runs
//! against a real `JetStream` server: `SITES_TEST_NATS_URL`, default `nats://127.0.0.1:4222`.

mod support;

use ratatoskr_sites::config::BusConfig;
use ratatoskr_sites::outbox::OutboxRelay;
use support::{Failure, Fixture, request};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

#[expect(
    clippy::disallowed_methods,
    reason = "the variable names where a test may reach JetStream; it is not part of the sites \
              configuration"
)]
fn nats_url() -> String {
    std::env::var("SITES_TEST_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned())
}

#[tokio::test]
async fn relay_publishes_each_row_once() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture
        .builder()
        .build(&request(Some(Uuid::now_v7())))
        .await?;

    let client = async_nats::connect(nats_url()).await?;
    let jetstream = async_nats::jetstream::new(client);
    let name = format!("sites_relay_test_{}", Uuid::now_v7().simple());
    let mut stream = jetstream
        .create_stream(async_nats::jetstream::stream::Config {
            name: name.clone(),
            subjects: vec![
                "evt.sites.release.published.v1".to_owned(),
                "evt.platform.operation.reported.v1".to_owned(),
            ],
            ..Default::default()
        })
        .await?;
    let relay = OutboxRelay::new(
        fixture.database.database.clone(),
        nats_url(),
        BusConfig::default(),
    );

    let first = relay.publish_pending().await;
    let second = relay.publish_pending().await;
    // A stop between the PubAck and the mark publishes a row again; JetStream drops the copy.
    sqlx::query("update sites.outbox set published_at = null")
        .execute(fixture.database.pool())
        .await?;
    let again = relay.publish_pending().await;
    let messages = stream.info().await?.state.messages;
    let unpublished: i64 =
        sqlx::query_scalar("select count(*) from sites.outbox where published_at is null")
            .fetch_one(fixture.database.pool())
            .await?;
    jetstream.delete_stream(&name).await?;

    assert_eq!(first?, 2, "the report and the release event are published");
    assert_eq!(second?, 0, "a published row is not published again");
    assert_eq!(again?, 2, "rows that lost their mark are sent again");
    assert_eq!(messages, 2, "JetStream keeps one copy of each event");
    assert_eq!(unpublished, 0, "every row is marked published");
    fixture.cleanup().await
}
