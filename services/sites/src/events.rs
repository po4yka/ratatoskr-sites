//! The event envelopes that Sites writes to its outbox.

use ratatoskr_error_contracts::{ErrorCode, ErrorEnvelope};
use ratatoskr_event_envelope::{EnvelopeSchemaVersion, EventEnvelope, EventPayload};
use ratatoskr_identifiers::{
    EntityRef, EventId, Extensions, OperationId, SafeMessage, TenantRef, WireTimestamp,
};
use ratatoskr_operation_contracts::{
    OperationReported, OperationResultKind, OperationResultRef, OperationStatus,
};
use ratatoskr_sites_contracts::{DraftChanged, Release, ReleasePublished, SiteId, Slug, SourceRef};
use uuid::Uuid;

/// The producer name on every envelope of this service.
pub const PRODUCER: &str = "ratatoskr-sites";

/// The subject of an operation report.
pub const OPERATION_REPORTED: &str = "evt.platform.operation.reported.v1";

/// The subject of the release event.
pub const RELEASE_PUBLISHED: &str = "evt.sites.release.published.v1";

/// The subject of the draft change event (SITE-027).
pub const DRAFT_CHANGED: &str = "evt.sites.draft.changed.v1";

/// The result kind of a build in its operation report.
const RELEASE_RESULT: &str = "sites.release";

/// An envelope could not be built. The message names the field, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the event envelope could not be built: {0}")]
pub struct EnvelopeBuildError(&'static str);

/// The end of a build, as its operation reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildReport<'a> {
    /// The build made or named the release `release_id`.
    Succeeded {
        /// The release id.
        release_id: &'a str,
    },
    /// The build made no release.
    Failed {
        /// The stable class.
        class: &'a str,
        /// What the owner reads.
        message: &'a str,
        /// Whether a new request can succeed.
        retryable: bool,
    },
}

/// The report of the build operation `operation_id` of `owner_id`.
///
/// # Errors
///
/// [`EnvelopeBuildError`] when a value does not satisfy the contract.
pub fn build_reported(
    event_id: Uuid,
    owner_id: Uuid,
    operation_id: Uuid,
    report: BuildReport<'_>,
) -> Result<EventEnvelope, EnvelopeBuildError> {
    let mut payload = OperationReported {
        operation_id: OperationId(operation_id),
        status: OperationStatus::Succeeded,
        stage: None,
        progress_percent: None,
        results: Vec::new(),
        error: None,
        warnings: Vec::new(),
        extensions: Extensions::new(),
    };
    match report {
        BuildReport::Succeeded { release_id } => payload.results.push(OperationResultRef {
            result_kind: OperationResultKind::parse(RELEASE_RESULT)
                .map_err(|_| EnvelopeBuildError("result_kind"))?,
            target: EntityRef::parse(&format!("site_release:{release_id}"))
                .map_err(|_| EnvelopeBuildError("target"))?,
            blob: None,
            ai_archive_import_summary: None,
            extensions: Extensions::new(),
        }),
        BuildReport::Failed {
            class,
            message,
            retryable,
        } => {
            payload.status = OperationStatus::Failed;
            payload.error = Some(ErrorEnvelope::new(
                ErrorCode::parse(class).map_err(|_| EnvelopeBuildError("code"))?,
                SafeMessage::parse(message).map_err(|_| EnvelopeBuildError("message"))?,
                retryable,
            ));
        }
    }
    payload
        .validate()
        .map_err(|_| EnvelopeBuildError("payload"))?;
    let operation = format!("operation:{operation_id}");
    envelope(
        event_id,
        OperationReported::event_type(),
        &operation,
        &operation,
        owner_id,
        &payload,
    )
}

/// `sites.release.published.v1` for the release `release_id` of `release`, owned by `owner_id`.
/// It carries the site id, the release id, the publication ids, the page count and the time; no
/// title, slug, path or content (workspace spec `site-publications`).
///
/// # Errors
///
/// [`EnvelopeBuildError`] when a value does not satisfy the contract.
pub fn release_published(
    event_id: Uuid,
    owner_id: Uuid,
    release_id: &str,
    release: &Release,
) -> Result<EventEnvelope, EnvelopeBuildError> {
    let payload = ReleasePublished {
        site_id: release.site_id.clone(),
        release_id: ratatoskr_identifiers::DigestHex::parse(release_id)
            .map_err(|_| EnvelopeBuildError("release_id"))?,
        publication_ids: release.publication_ids.clone(),
        page_count: u32::try_from(release.pages.len())
            .map_err(|_| EnvelopeBuildError("page_count"))?,
        published_at: WireTimestamp::now(),
    };
    payload
        .validate()
        .map_err(|_| EnvelopeBuildError("payload"))?;
    envelope(
        event_id,
        ReleasePublished::event_type(),
        &format!("site:{}", release.site_id.as_str()),
        &format!("event:{event_id}"),
        owner_id,
        &payload,
    )
}

/// `sites.draft.changed.v1`: the note `source_ref` of `site_id`, owned by `owner_id`, has the new
/// revision `revision`, and the page `slug` when the current release publishes it. It carries no
/// content and no vault path (workspace design D5 of `add-site-owner-editing-and-live`).
///
/// # Errors
///
/// [`EnvelopeBuildError`] when a value does not satisfy the contract.
pub fn draft_changed(
    event_id: Uuid,
    owner_id: Uuid,
    site_id: &str,
    source_ref: &str,
    revision: &str,
    slug: Option<&str>,
) -> Result<EventEnvelope, EnvelopeBuildError> {
    let payload = DraftChanged {
        site_id: SiteId::parse(site_id).map_err(|_| EnvelopeBuildError("site_id"))?,
        source_ref: SourceRef::parse(source_ref).map_err(|_| EnvelopeBuildError("source_ref"))?,
        revision: ratatoskr_identifiers::DigestHex::parse(revision)
            .map_err(|_| EnvelopeBuildError("revision"))?,
        slug: slug
            .map(Slug::parse)
            .transpose()
            .map_err(|_| EnvelopeBuildError("slug"))?,
    };
    envelope(
        event_id,
        DraftChanged::event_type(),
        &format!("site:{site_id}"),
        &format!("event:{event_id}"),
        owner_id,
        &payload,
    )
}

/// An envelope of this service with `payload`, owned by `owner_id`.
///
/// # Errors
///
/// [`EnvelopeBuildError`] when a value does not satisfy the contract.
pub fn envelope<P: EventPayload + serde::Serialize>(
    event_id: Uuid,
    event_type: ratatoskr_event_envelope::EventType,
    aggregate_id: &str,
    correlation_id: &str,
    owner_id: Uuid,
    payload: &P,
) -> Result<EventEnvelope, EnvelopeBuildError> {
    let mut envelope = EventEnvelope {
        event_id: EventId(event_id),
        event_type,
        occurred_at: WireTimestamp::now(),
        producer: ratatoskr_event_envelope::ProducerName::parse(PRODUCER)
            .map_err(|_| EnvelopeBuildError("producer"))?,
        aggregate_id: EntityRef::parse(aggregate_id)
            .map_err(|_| EnvelopeBuildError("aggregate_id"))?,
        correlation_id: EntityRef::parse(correlation_id)
            .map_err(|_| EnvelopeBuildError("correlation_id"))?,
        causation_id: None,
        tenant_id: Some(
            TenantRef::parse(&format!("user:{owner_id}"))
                .map_err(|_| EnvelopeBuildError("tenant_id"))?,
        ),
        schema_version: EnvelopeSchemaVersion::CURRENT,
        payload: serde_json::Map::new(),
        extensions: Extensions::new(),
    };
    envelope
        .set_payload(payload)
        .map_err(|_| EnvelopeBuildError("payload"))?;
    Ok(envelope)
}
