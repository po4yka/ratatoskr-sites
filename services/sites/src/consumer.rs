//! The consumers of the four durables that Edge provisions for the sites service (workspace spec
//! `fleet-bus-topology`).
//!
//! Edge creates the streams and the durables; this service never creates either. Each consumer
//! looks its durable up by name and compares its filter, its acknowledgement policy and its ack
//! wait with [`DurableSpec`]. A durable that is missing or differs is logged, and the consumer
//! tries again after [`RETRY_INTERVAL`]. The connection uses the inbox prefix `_INBOX_sites`, the
//! only one that the `SITES` identity may subscribe to.
//!
//! A message is acknowledged after the [`Inbound`] handler committed its effect, so a stop between
//! the two delivers it again, and the inbox or the key of the effect makes the repeat a repeat.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::consumer::{AckPolicy, Consumer, pull};
use async_nats::jetstream::{self, AckKind, Context};
use futures_util::StreamExt as _;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::BusConfig;

/// The command stream that Edge owns.
pub const COMMAND_STREAM: &str = "ratatoskr_commands";

/// The event stream that Edge owns.
pub const EVENT_STREAM: &str = "ratatoskr_events";

/// How long one request to the server may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a consumer waits before it starts again after a failure.
const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// How many messages one pull asks for: a bound on the messages that wait in memory.
const BATCH: usize = 16;

/// How long the server waits before it delivers again a message whose effect failed.
const REDELIVERY_DELAY: Duration = Duration::from_secs(5);

/// One of the four durables of the sites service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durable {
    /// `ratatoskr_sites_release_builds`: `cmd.sites.release.build_requested.v1`.
    ReleaseBuilds,
    /// `ratatoskr_sites_note_changes`: `evt.notes.file.changed.v1`.
    NoteChanges,
    /// `ratatoskr_sites_vault_disconnections`: `evt.notes.vault.disconnected.v1`.
    VaultDisconnections,
    /// `ratatoskr_sites_approvals`: `evt.platform.site_approval.changed.v1`.
    Approvals,
}

/// The stream, the name, the filter and the ack wait of one durable, as Edge provisions it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSpec {
    /// The stream that holds the durable.
    pub stream: String,
    /// The durable name.
    pub durable: String,
    /// The one subject that the durable delivers.
    pub filter: String,
    /// How long the server waits for an acknowledgement.
    pub ack_wait: Duration,
}

impl DurableSpec {
    fn of(stream: &str, durable: &str, filter: &str, ack_wait_seconds: u64) -> Self {
        Self {
            stream: stream.to_owned(),
            durable: durable.to_owned(),
            filter: filter.to_owned(),
            ack_wait: Duration::from_secs(ack_wait_seconds),
        }
    }
}

/// The four durables of the sites service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Durables {
    /// The build commands that Edge publishes.
    pub release_builds: DurableSpec,
    /// The file changes of notes.
    pub note_changes: DurableSpec,
    /// The vault disconnections of notes.
    pub vault_disconnections: DurableSpec,
    /// The site approvals of Platform.
    pub approvals: DurableSpec,
}

impl Durables {
    /// The durables as Edge provisions them (`ratatoskr-platform`, `crates/eventing/src/stream.rs`):
    /// a build waits 300 s for its acknowledgement, a note change 120 s, the others 30 s.
    #[must_use]
    pub fn production() -> Self {
        Self {
            release_builds: DurableSpec::of(
                COMMAND_STREAM,
                "ratatoskr_sites_release_builds",
                "cmd.sites.release.build_requested.v1",
                300,
            ),
            note_changes: DurableSpec::of(
                EVENT_STREAM,
                "ratatoskr_sites_note_changes",
                "evt.notes.file.changed.v1",
                120,
            ),
            vault_disconnections: DurableSpec::of(
                EVENT_STREAM,
                "ratatoskr_sites_vault_disconnections",
                "evt.notes.vault.disconnected.v1",
                30,
            ),
            approvals: DurableSpec::of(
                EVENT_STREAM,
                "ratatoskr_sites_approvals",
                "evt.platform.site_approval.changed.v1",
                30,
            ),
        }
    }

    /// The spec of `durable`.
    #[must_use]
    pub const fn spec(&self, durable: Durable) -> &DurableSpec {
        match durable {
            Durable::ReleaseBuilds => &self.release_builds,
            Durable::NoteChanges => &self.note_changes,
            Durable::VaultDisconnections => &self.vault_disconnections,
            Durable::Approvals => &self.approvals,
        }
    }
}

/// What the consumer does with a message after its handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// The effect is committed: acknowledge.
    Done,
    /// The effect failed: deliver the message again after [`REDELIVERY_DELAY`].
    Retry,
    /// The message is not a valid envelope of its durable: never deliver it again.
    Reject,
}

/// The handler of the messages of the durables.
pub trait Inbound: Send + Sync + 'static {
    /// Apply the message `payload` of `durable`.
    fn handle(&self, durable: Durable, payload: &[u8]) -> impl Future<Output = Disposition> + Send;
}

/// The bus that the consumers connect to.
#[derive(Debug, Clone)]
pub struct BusTarget {
    /// The server.
    pub url: String,
    /// The identity.
    pub bus: BusConfig,
}

/// Start one consumer for each of `durables`, which hands each message to `handler` until `stop`.
pub fn spawn<H: Inbound>(
    target: &BusTarget,
    specs: &Durables,
    durables: &[Durable],
    handler: &Arc<H>,
    stop: &CancellationToken,
) -> Vec<JoinHandle<()>> {
    durables
        .iter()
        .map(|&durable| {
            let target = target.clone();
            let spec = specs.spec(durable).clone();
            let handler = Arc::clone(handler);
            let stop = stop.clone();
            tokio::spawn(async move { run(target, durable, spec, handler, stop).await })
        })
        .collect()
}

async fn run<H: Inbound>(
    target: BusTarget,
    durable: Durable,
    spec: DurableSpec,
    handler: Arc<H>,
    stop: CancellationToken,
) {
    loop {
        match consume(&target, durable, &spec, handler.as_ref(), &stop).await {
            Ok(()) => return,
            Err(error) => {
                tracing::warn!(%error, durable = %spec.durable, "a consumer stopped; it starts again");
            }
        }
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = tokio::time::sleep(RETRY_INTERVAL) => {}
        }
    }
}

/// The durable is missing, cannot be reached, or differs from what Edge provisions.
#[derive(Debug, thiserror::Error)]
#[error("{0}; start ratatoskr-edge first so that it provisions the durable")]
pub struct DurableError(String);

/// Look `spec` up and compare it with what Edge provisions.
async fn lookup(
    context: &Context,
    spec: &DurableSpec,
) -> Result<Consumer<pull::Config>, DurableError> {
    let consumer = context
        .get_consumer_from_stream::<pull::Config, _, _>(&spec.durable, &spec.stream)
        .await
        .map_err(|error| DurableError(format!("the durable cannot be read: {error}")))?;
    let config = &consumer.cached_info().config;
    let matches = config.durable_name.as_deref() == Some(spec.durable.as_str())
        && config.filter_subject == spec.filter
        && config.ack_policy == AckPolicy::Explicit
        && config.ack_wait == spec.ack_wait;
    if matches {
        Ok(consumer)
    } else {
        Err(DurableError(
            "the durable differs from its specification".to_owned(),
        ))
    }
}

/// Consume `spec` until `stop` (`Ok`) or until the bus or the durable fails.
async fn consume<H: Inbound>(
    target: &BusTarget,
    durable: Durable,
    spec: &DurableSpec,
    handler: &H,
    stop: &CancellationToken,
) -> Result<(), DurableError> {
    let client = crate::outbox::connect(&target.url, &target.bus)
        .await
        .map_err(|error| DurableError(error.to_string()))?;
    let context = jetstream::ContextBuilder::new()
        .timeout(REQUEST_TIMEOUT)
        .build(client);
    let consumer = lookup(&context, spec).await?;
    let mut messages = consumer
        .stream()
        .max_messages_per_batch(BATCH)
        .messages()
        .await
        .map_err(|error| DurableError(format!("the durable cannot be pulled: {error}")))?;
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => return Ok(()),
            message = messages.next() => match message {
                Some(Ok(message)) => {
                    let kind = match handler.handle(durable, &message.payload).await {
                        Disposition::Done => AckKind::Ack,
                        Disposition::Retry => AckKind::Nak(Some(REDELIVERY_DELAY)),
                        Disposition::Reject => AckKind::Term,
                    };
                    if let Err(error) = message.ack_with(kind).await {
                        tracing::warn!(%error, durable = %spec.durable, "a message could not be acknowledged");
                    }
                }
                Some(Err(error)) => {
                    return Err(DurableError(format!("the durable stopped delivering: {error}")));
                }
                None => return Err(DurableError("the durable stopped delivering".to_owned())),
            },
        }
    }
}
