//! SITE-025 task 5.18: a release is immutable and named by its content (workspace spec
//! `site-publications`, requirement "A release is immutable and named by its content"), and a
//! build reports its release id through its operation.

mod support;

use std::time::Duration;

use ratatoskr_sites::release_build::BuildFailure;
use ratatoskr_sites_contracts::Release;
use ratatoskr_sites_release::sha256_hex;
use serde_json::Value;
use support::{Failure, Fixture, SITE, blob_path, request};
use uuid::Uuid;

type Outcome = Result<(), Failure>;

/// The RFC 8785 form of a JSON value without numbers other than integers: object members sorted by
/// the UTF-16 code units of their names, no white space. Written here, apart from the service, so
/// the test does not check the service with its own code.
fn rfc8785(value: &Value) -> Result<String, Failure> {
    Ok(match value {
        Value::Object(object) => {
            let mut members: Vec<(&String, &Value)> = object.iter().collect();
            members.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
            let mut text = String::from("{");
            for (index, (name, member)) in members.into_iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&serde_json::to_string(name)?);
                text.push(':');
                text.push_str(&rfc8785(member)?);
            }
            text.push('}');
            text
        }
        Value::Array(items) => {
            let items: Result<Vec<String>, Failure> = items.iter().map(rfc8785).collect();
            format!("[{}]", items?.join(","))
        }
        Value::Number(number) if number.is_f64() => return Err("a release holds no float".into()),
        other => serde_json::to_string(other)?,
    })
}

#[tokio::test]
async fn unchanged_content_makes_no_new_release() -> Outcome {
    let fixture = Fixture::new().await?;
    let builder = fixture.builder();

    let first = builder.build(&request(None)).await?;
    let second = builder.build(&request(None)).await?;

    assert_eq!(
        fixture.release_count().await?,
        1,
        "a second build of unchanged content stores no release"
    );
    assert_eq!(
        second.release_id, first.release_id,
        "the second build names the first release"
    );
    assert!(first.new_release && !second.new_release);
    fixture.cleanup().await
}

#[tokio::test]
async fn earlier_release_is_unchanged() -> Outcome {
    let fixture = Fixture::new().await?;
    let builder = fixture.builder();
    let first = builder.build(&request(None)).await?;

    fixture.change(
        "other.md",
        "---\npublish: true\n---\nThe other page, changed.\n",
    )?;
    let second = builder.build(&request(None)).await?;
    assert_ne!(
        second.release_id, first.release_id,
        "a changed file makes a new release"
    );
    assert_eq!(fixture.release_count().await?, 2);

    let document = fixture.document(&first.release_id).await?;
    assert_eq!(
        sha256_hex(&document),
        first.release_id,
        "the earlier document still hashes to its release id"
    );
    let release: Release = serde_json::from_slice(&document)?;
    for page in &release.pages {
        let bytes = std::fs::read(blob_path(
            fixture.sites_root.path(),
            page.page_sha256.as_str(),
        )?)?;
        assert_eq!(
            sha256_hex(&bytes),
            page.page_sha256.as_str(),
            "the page {} of the earlier release is unchanged",
            page.slug.as_str()
        );
    }
    fixture.cleanup().await
}

#[tokio::test]
async fn build_reports_its_release_id() -> Outcome {
    let fixture = Fixture::new().await?;
    let operation = Uuid::now_v7();
    let outcome = fixture.builder().build(&request(Some(operation))).await?;

    let document = fixture.document(&outcome.release_id).await?;
    assert_eq!(
        outcome.release_id,
        sha256_hex(&document),
        "the release id is the SHA-256 of the stored document"
    );
    let value: Value = serde_json::from_slice(&document)?;
    assert_eq!(
        String::from_utf8(document.clone())?,
        rfc8785(&value)?,
        "the stored document is the RFC 8785 form of the release"
    );
    let release: Release = serde_json::from_slice(&document)?;
    assert!(!release.pages.is_empty(), "the release has pages");

    let reports: Vec<Value> = sqlx::query_scalar(
        "select payload from sites.outbox where subject = 'evt.platform.operation.reported.v1'",
    )
    .fetch_all(fixture.database.pool())
    .await?;
    assert_eq!(reports.len(), 1, "the build reports its operation once");
    let report = reports
        .first()
        .and_then(|envelope| envelope.get("payload"))
        .ok_or("a payload")?;
    assert_eq!(
        report.get("operation_id"),
        Some(&Value::from(operation.to_string()))
    );
    assert_eq!(report.get("status"), Some(&Value::from("succeeded")));
    assert_eq!(
        report.pointer("/results/0/target"),
        Some(&Value::from(format!("site_release:{}", outcome.release_id))),
        "the report names the release id: {report}"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn blob_over_the_bound_fails_with_blob_length() -> Outcome {
    let fixture = Fixture::new().await?;
    // The builder of the fixture reads without a bound of its own; the service applies its bound.
    let over = vec![0_u8; usize::try_from(support::MAX_NOTE_BYTES + 1)?];
    fixture.replace(
        "attachments/2026/passport-scan-blurred.png",
        &over,
        "image/png",
    )?;

    let built = fixture.builder().build(&request(None)).await;
    assert_eq!(
        built.as_ref().map_err(BuildFailure::class).err().as_deref(),
        Some("sites.blob_length"),
        "a blob of more than 64 MiB fails the build before it is read: {:?}",
        built.map(|outcome| outcome.release_id)
    );
    assert_eq!(
        fixture.release_count().await?,
        0,
        "the build makes no release"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn long_failure_detail_is_still_reported() -> Outcome {
    let fixture = Fixture::new().await?;
    // Two notes whose slugs collide, with paths so long that the detail of the failure is longer
    // than a SafeMessage may be.
    let folder = "Long".repeat(150);
    fixture.add_note(&format!("{folder}/A B.md"), "One.\n")?;
    fixture.add_note(&format!("{}/a-b.md", folder.to_lowercase()), "Two.\n")?;
    let operation = Uuid::now_v7();

    let built = fixture.builder().build(&request(Some(operation))).await;
    assert_eq!(
        built.as_ref().map_err(BuildFailure::class).err().as_deref(),
        Some("sites.slug_collision")
    );
    let reports = fixture.reports().await?;
    assert_eq!(reports.len(), 1, "the failure is reported once");
    let report = reports.first().ok_or("a report")?;
    assert_eq!(report.get("status"), Some(&Value::from("failed")));
    assert_eq!(
        report.pointer("/error/code"),
        Some(&Value::from("sites.slug_collision")),
        "{report}"
    );
    let message = report
        .pointer("/error/message")
        .and_then(Value::as_str)
        .ok_or("the report has a message")?;
    assert!(
        (1..=1024).contains(&message.chars().count()),
        "the message is a safe message: {} characters",
        message.chars().count()
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn report_failure_keeps_the_build_error() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.add_note("Notes/A B.md", "One.\n")?;
    fixture.add_note("notes/a-b.md", "Two.\n")?;
    // The outbox cannot take the report.
    sqlx::query("alter table sites.outbox rename to outbox_gone")
        .execute(fixture.database.pool())
        .await?;

    let built = fixture
        .builder()
        .build(&request(Some(Uuid::now_v7())))
        .await;
    assert_eq!(
        built.as_ref().map_err(BuildFailure::class).err().as_deref(),
        Some("sites.slug_collision"),
        "a report that cannot be written does not replace the error of the build"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn build_that_loses_the_race_switches_nothing() -> Outcome {
    let fixture = Fixture::new().await?;
    let first = fixture.builder().build(&request(None)).await?;

    // A build of the same content finds the first release current and waits before the lock.
    let late_builder = fixture.builder();
    let pause = late_builder.pause_before_lock();
    let late = tokio::spawn(async move {
        late_builder
            .build(&request(None))
            .await
            .map_err(|error| error.to_string())
    });
    tokio::time::timeout(Duration::from_secs(10), pause.reached.notified())
        .await
        .map_err(|_| "the late build did not reach the lock within 10 s")?;

    // Meanwhile another build makes another release current.
    fixture.change(
        "other.md",
        "---\npublish: true\n---\nThe other page, changed.\n",
    )?;
    let second = fixture.builder().build(&request(None)).await?;
    assert_ne!(second.release_id, first.release_id);

    pause.release.notify_one();
    let late = late.await??;
    let (current, _) =
        ratatoskr_sites::release_read::current_release(&fixture.database.database, SITE)
            .await?
            .ok_or("the site has a release")?;
    assert_eq!(
        current, second.release_id,
        "the build that lost the race does not switch the site back"
    );
    assert!(!late.new_release, "it makes no new release: {late:?}");
    let events: i64 = sqlx::query_scalar(
        "select count(*) from sites.outbox where subject = 'evt.sites.release.published.v1'",
    )
    .fetch_one(fixture.database.pool())
    .await?;
    assert_eq!(events, 2, "and it announces nothing");
    fixture.cleanup().await
}
