//! A real `JetStream` for the consumer tests: `SITES_TEST_NATS_URL`, default
//! `nats://127.0.0.1:4222`.
//!
//! Each test provisions its own command and event streams, with subjects under a prefix of its
//! own, and the four durables on them as Edge provisions them. So tests do not share a subject with
//! each other or with a stream `ratatoskr_events`.

use std::time::Duration;

use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, pull};
use async_nats::jetstream::{self, Context, stream};
use ratatoskr_event_envelope::{CommandEnvelope, EnvelopeSchemaVersion, EventEnvelope};
use ratatoskr_identifiers::{
    CommandId, EntityRef, EventId, Extensions, OperationId, TenantRef, UserId, WireTimestamp,
};
use ratatoskr_sites::config::BusConfig;
use ratatoskr_sites::consumer::{BusTarget, Durable, DurableSpec, Durables};
use ratatoskr_sites_contracts::{ReleaseBuildRequested, SiteId};
use uuid::Uuid;

use super::Failure;

/// The fixed time of every test envelope: a test never reads the wall clock.
const OCCURRED_AT: &str = "2026-10-15T08:00:00Z";

/// How long a test waits for the consumers.
const PATIENCE: Duration = Duration::from_secs(30);

#[expect(
    clippy::disallowed_methods,
    reason = "the variable names where a test may reach JetStream; it is not part of the sites \
              configuration"
)]
pub(crate) fn nats_url() -> String {
    std::env::var("SITES_TEST_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned())
}

/// The streams and durables of one test.
pub(crate) struct TestBus {
    pub(crate) jetstream: Context,
    prefix: String,
    pub(crate) durables: Durables,
}

impl TestBus {
    pub(crate) async fn new() -> Result<Self, Failure> {
        let jetstream = jetstream::new(async_nats::connect(nats_url()).await?);
        let prefix = format!("t{}", Uuid::now_v7().simple());
        for (name, class) in [("commands", "cmd"), ("events", "evt")] {
            jetstream
                .create_stream(stream::Config {
                    name: format!("{prefix}_{name}"),
                    subjects: vec![format!("{prefix}.{class}.>")],
                    ..Default::default()
                })
                .await?;
        }
        let spec = |stream: &str, durable: &str, subject: &str, seconds: u64| DurableSpec {
            stream: format!("{prefix}_{stream}"),
            durable: durable.to_owned(),
            filter: format!("{prefix}.{subject}"),
            ack_wait: Duration::from_secs(seconds),
        };
        let durables = Durables {
            release_builds: spec(
                "commands",
                "ratatoskr_sites_release_builds",
                "cmd.sites.release.build_requested.v1",
                300,
            ),
            note_changes: spec(
                "events",
                "ratatoskr_sites_note_changes",
                "evt.notes.file.changed.v1",
                120,
            ),
            vault_disconnections: spec(
                "events",
                "ratatoskr_sites_vault_disconnections",
                "evt.notes.vault.disconnected.v1",
                30,
            ),
            approvals: spec(
                "events",
                "ratatoskr_sites_approvals",
                "evt.platform.site_approval.changed.v1",
                30,
            ),
        };
        for durable in ALL {
            let spec = durables.spec(durable);
            jetstream
                .create_consumer_on_stream(
                    pull::Config {
                        durable_name: Some(spec.durable.clone()),
                        filter_subject: spec.filter.clone(),
                        ack_policy: AckPolicy::Explicit,
                        ack_wait: spec.ack_wait,
                        deliver_policy: DeliverPolicy::All,
                        max_deliver: -1,
                        ..Default::default()
                    },
                    spec.stream.as_str(),
                )
                .await?;
        }
        Ok(Self {
            jetstream,
            prefix,
            durables,
        })
    }

    /// The server and the identity that the service connects with.
    pub(crate) fn target() -> BusTarget {
        BusTarget {
            url: nats_url(),
            bus: BusConfig::default(),
        }
    }

    /// Publish `body` on `subject` under the prefix of this test.
    pub(crate) async fn publish(&self, subject: &str, body: Vec<u8>) -> Result<(), Failure> {
        self.jetstream
            .publish(format!("{}.{subject}", self.prefix), body.into())
            .await?
            .await?;
        Ok(())
    }

    /// Wait until `durable` has delivered and acknowledged every message.
    pub(crate) async fn settled(&self, durable: Durable) -> Result<(), Failure> {
        let spec = self.durables.spec(durable);
        let waited = tokio::time::timeout(PATIENCE, async {
            loop {
                let mut consumer: jetstream::consumer::Consumer<pull::Config> = self
                    .jetstream
                    .get_consumer_from_stream(&spec.durable, &spec.stream)
                    .await?;
                let info = consumer.info().await?;
                if info.num_pending == 0 && info.num_ack_pending == 0 {
                    return Ok::<(), Failure>(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        waited.map_err(|_| format!("{} did not settle", spec.durable))?
    }

    pub(crate) async fn cleanup(self) -> Result<(), Failure> {
        for name in ["commands", "events"] {
            self.jetstream
                .delete_stream(format!("{}_{name}", self.prefix))
                .await?;
        }
        Ok(())
    }
}

/// The four durables.
const ALL: [Durable; 4] = [
    Durable::ReleaseBuilds,
    Durable::NoteChanges,
    Durable::VaultDisconnections,
    Durable::Approvals,
];

/// An event envelope of `producer` with `payload`, owned by `owner`, at the fixed test time.
pub(crate) fn event<P: ratatoskr_event_envelope::EventPayload + serde::Serialize>(
    producer: &str,
    owner: Uuid,
    payload: &P,
) -> Result<Vec<u8>, Failure> {
    let event_id = Uuid::now_v7();
    let mut envelope = EventEnvelope {
        event_id: EventId(event_id),
        event_type: P::event_type(),
        occurred_at: WireTimestamp::parse(OCCURRED_AT)?,
        producer: ratatoskr_event_envelope::ProducerName::parse(producer)?,
        aggregate_id: EntityRef::parse(&format!("event:{event_id}"))?,
        correlation_id: EntityRef::parse(&format!("event:{event_id}"))?,
        causation_id: None,
        tenant_id: Some(TenantRef::of_user(UserId(owner))),
        schema_version: EnvelopeSchemaVersion::CURRENT,
        payload: serde_json::Map::new(),
        extensions: Extensions::new(),
    };
    envelope.set_payload(payload)?;
    Ok(serde_json::to_vec(&envelope)?)
}

/// The build command of Edge for `site` and the operation `operation_id`, owned by `owner`.
pub(crate) fn build_command(
    site: &str,
    owner: Uuid,
    operation_id: Uuid,
) -> Result<Vec<u8>, Failure> {
    let command_id = Uuid::now_v7();
    let mut envelope = CommandEnvelope {
        command_id: CommandId(command_id),
        command_type: ratatoskr_event_envelope::CommandType::parse(
            <ReleaseBuildRequested as ratatoskr_event_envelope::CommandPayload>::COMMAND_TYPE,
        )?,
        issued_at: WireTimestamp::parse(OCCURRED_AT)?,
        producer: ratatoskr_event_envelope::ProducerName::parse("ratatoskr-edge")?,
        aggregate_id: EntityRef::parse(&format!("command:{command_id}"))?,
        correlation_id: EntityRef::parse(&format!("operation:{operation_id}"))?,
        causation_id: None,
        tenant_id: Some(TenantRef::of_user(UserId(owner))),
        schema_version: EnvelopeSchemaVersion::CURRENT,
        payload: serde_json::Map::new(),
        extensions: Extensions::new(),
    };
    envelope.set_payload(&ReleaseBuildRequested {
        operation_id: OperationId(operation_id),
        site_id: SiteId::parse(site)?,
    })?;
    Ok(serde_json::to_vec(&envelope)?)
}

/// The builds of a running service: it counts each build when it starts, and a build waits for a
/// permit of `gate` before it runs.
#[derive(Debug, Clone)]
pub(crate) struct Counted {
    inner: std::sync::Arc<ratatoskr_sites::release_build::ReleaseBuilder<super::StubIndex>>,
    builds: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    gate: std::sync::Arc<tokio::sync::Semaphore>,
}

impl ratatoskr_sites::worker::SiteBuilds for Counted {
    async fn build(
        &self,
        job: &ratatoskr_sites::release_build::BuildJob,
    ) -> Result<
        ratatoskr_sites::release_build::BuildOutcome,
        ratatoskr_sites::release_build::BuildFailure,
    > {
        self.builds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(permit) = self.gate.acquire().await {
            permit.forget();
        }
        self.inner.build_job(job).await
    }
}

/// The consumers and the build worker of a service under test.
pub(crate) struct Running {
    pub(crate) shared: ratatoskr_sites::triggers::Shared,
    stop: tokio_util::sync::CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    builds: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    gate: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Running {
    /// How many builds started.
    pub(crate) fn builds(&self) -> usize {
        self.builds.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Let `count` more builds run.
    pub(crate) fn open(&self, count: usize) {
        self.gate.add_permits(count);
    }

    /// Wait until `count` builds started.
    pub(crate) async fn started(&self, count: usize) -> Result<(), Failure> {
        tokio::time::timeout(PATIENCE, async {
            while self.builds() < count {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| format!("{count} builds did not start; {} did", self.builds()).into())
    }

    pub(crate) async fn stop(self) {
        self.stop.cancel();
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

impl super::Fixture {
    /// Start the consumers of every durable of `bus` and the build worker. `permits` builds may
    /// run before the test opens the gate further.
    pub(crate) async fn start(&self, bus: &TestBus, permits: usize) -> Result<Running, Failure> {
        let shared = ratatoskr_sites::triggers::Shared {
            database: self.database.database.clone(),
            servable: ratatoskr_sites::servable::Servable::load(&self.database.database).await?,
        };
        let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(permits));
        let counted = Counted {
            inner: std::sync::Arc::new(self.builder()),
            builds: std::sync::Arc::clone(&builds),
            gate: std::sync::Arc::clone(&gate),
        };
        let stop = tokio_util::sync::CancellationToken::new();
        let tasks = ratatoskr_sites::triggers::start(
            &shared,
            self.index.clone(),
            counted,
            &ratatoskr_sites::triggers::Consumers {
                target: &TestBus::target(),
                specs: &bus.durables,
                durables: &ALL,
            },
            &stop,
        );
        Ok(Running {
            shared,
            stop,
            tasks,
            builds,
            gate,
        })
    }
}

/// `evt.platform.site_approval.changed.v1` of Platform for `site`, owned by `owner`.
pub(crate) fn approval(
    site: &str,
    digest: &str,
    capabilities: &[&str],
    hostname: &str,
    approved: bool,
    owner: Uuid,
) -> Result<Vec<u8>, Failure> {
    let payload: ratatoskr_sites_contracts::SiteApprovalChanged =
        serde_json::from_value(serde_json::json!({
            "site_id": site,
            "manifest_sha256": digest,
            "capabilities": capabilities,
            "hostname": hostname,
            "state": if approved { "approved" } else { "withdrawn" },
        }))?;
    event("ratatoskr-edge", owner, &payload)
}

/// `evt.notes.vault.disconnected.v1` of notes for `connection`, owned by `owner`.
pub(crate) fn disconnection(connection: Uuid, owner: Uuid) -> Result<Vec<u8>, Failure> {
    let payload: ratatoskr_notes_contracts::VaultDisconnected =
        serde_json::from_value(serde_json::json!({
            "connection_id": connection.to_string(),
            "channel": "plugin",
            "vault_display_name": "Fixture vault",
        }))?;
    event("ratatoskr-notes", owner, &payload)
}

/// The operation report of `operation_id` in the outbox, if there is one.
pub(crate) async fn report_of(
    fixture: &super::Fixture,
    operation_id: Uuid,
) -> Result<Option<serde_json::Value>, Failure> {
    Ok(fixture.reports().await?.into_iter().find(|report| {
        report
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            == Some(operation_id.to_string().as_str())
    }))
}

/// Publish a build command of Edge for the operation `operation_id`, and wait for its report.
pub(crate) async fn requested_build(
    fixture: &super::Fixture,
    bus: &TestBus,
    operation_id: Uuid,
) -> Result<serde_json::Value, Failure> {
    bus.publish(
        "cmd.sites.release.build_requested.v1",
        build_command(super::SITE, fixture.owner, operation_id)?,
    )
    .await?;
    let report = tokio::time::timeout(PATIENCE, async {
        loop {
            if let Some(report) = report_of(fixture, operation_id).await? {
                return Ok::<_, Failure>(report);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "the build reported nothing")??;
    Ok(report)
}

/// A manifest of the fixture site `site-r`.
#[derive(Debug, Clone)]
pub(crate) struct TestManifest {
    pub(crate) hostname: String,
    pub(crate) publications: Vec<String>,
    pub(crate) capabilities: Vec<String>,
}

impl TestManifest {
    /// A manifest with `hostname`, `publications` and `capabilities`.
    pub(crate) fn of(hostname: &str, publications: &[&str], capabilities: &[&str]) -> Self {
        Self {
            hostname: hostname.to_owned(),
            publications: publications.iter().map(|&item| item.to_owned()).collect(),
            capabilities: capabilities.iter().map(|&item| item.to_owned()).collect(),
        }
    }

    /// The TOML bytes of `ratatoskr-site.toml`.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let list = |items: &[String]| {
            items
                .iter()
                .map(|item| format!("\"{item}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "site_id = \"{}\"\nhostname = \"{}\"\npublications = [{}]\ncapabilities = [{}]\n",
            super::SITE,
            self.hostname,
            list(&self.publications),
            list(&self.capabilities)
        )
        .into_bytes()
    }

    /// The digest that an approval names.
    pub(crate) fn digest(&self) -> String {
        ratatoskr_sites_manifest::digest(&self.bytes())
    }
}

/// Publish an approval change of Platform for `manifest` and wait until the service consumed it.
pub(crate) async fn approve(
    bus: &TestBus,
    fixture: &super::Fixture,
    manifest: &TestManifest,
    approved: bool,
) -> Result<(), Failure> {
    let capabilities: Vec<&str> = manifest.capabilities.iter().map(String::as_str).collect();
    bus.publish(
        "evt.platform.site_approval.changed.v1",
        approval(
            super::SITE,
            &manifest.digest(),
            &capabilities,
            &manifest.hostname,
            approved,
            fixture.owner,
        )?,
    )
    .await?;
    bus.settled(Durable::Approvals).await
}
