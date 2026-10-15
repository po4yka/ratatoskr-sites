//! SITE-025 task 5.34: search covers only the current release, and its parameters are bounded
//! (workspace spec `site-content-delivery`).

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::http::{Answer, anonymous, call};
use support::{Failure, Fixture, SITE, request};

type Outcome = Result<(), Failure>;

/// Every canary of the privacy vault: each comes only from a file that the publication does not
/// select (`fixtures/privacy-vault/README.md`).
const CANARIES: [&str; 21] = [
    "zq7diaryblock",
    "zq7diarybody",
    "zq7diaryfield",
    "zq7diaryheading",
    "zq7diaryname",
    "zq7diarytag",
    "zq7diarytitle",
    "zq7planblock",
    "zq7planbody",
    "zq7planfield",
    "zq7planheading",
    "zq7plannote",
    "zq7plantag",
    "zq7plantitle",
    "zq7salaryblock",
    "zq7salarybody",
    "zq7salaryfield",
    "zq7salaryheading",
    "zq7salaryname",
    "zq7salarytag",
    "zq7salarytitle",
];

/// The answer of the search route for the query string `query`.
async fn search(fixture: &Fixture, query: &str) -> Result<Answer, Failure> {
    call(
        &fixture.api().await?,
        anonymous(
            Method::GET,
            &format!("/v1/sites/{SITE}/content/search?{query}"),
        )?,
    )
    .await
}

/// The slugs of the results of `q`.
async fn slugs(fixture: &Fixture, q: &str) -> Result<Vec<String>, Failure> {
    let answer = search(fixture, &format!("q={q}")).await?;
    assert_eq!(answer.status, StatusCode::OK, "{q}: the search answers");
    let body = answer.json()?;
    Ok(body
        .get("results")
        .and_then(Value::as_array)
        .ok_or("the answer has results")?
        .iter()
        .filter_map(|result| result.get("slug")?.as_str().map(str::to_owned))
        .collect())
}

#[tokio::test]
async fn search_never_returns_unselected_content() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    assert_eq!(
        slugs(&fixture, "target").await?,
        ["other"],
        "a word of a published page is found"
    );
    for canary in CANARIES {
        let answer = search(&fixture, &format!("q={canary}")).await?;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(
            answer.json()?,
            json!({ "results": [], "has_more": false }),
            "{canary} of an unselected file matches nothing"
        );
    }
    fixture.cleanup().await
}

#[tokio::test]
async fn search_follows_the_current_release() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.change(
        "other.md",
        "---\ntitle: Other page\npublish: true\n---\n# Other\n\nA zebrafish swims here.\n",
    )?;
    fixture.builder().build(&request(None)).await?;
    assert_eq!(
        slugs(&fixture, "zebrafish").await?,
        ["other"],
        "the page that holds the word is found"
    );

    fixture.remove("other.md")?;
    fixture.builder().build(&request(None)).await?;
    assert_eq!(
        slugs(&fixture, "zebrafish").await?,
        Vec::<String>::new(),
        "a page that the current release no longer holds is not found"
    );
    fixture.cleanup().await
}

#[tokio::test]
async fn over_long_query_is_refused_before_search() -> Outcome {
    let fixture = Fixture::new().await?;
    fixture.builder().build(&request(None)).await?;
    let long = "a".repeat(257);
    for query in [
        format!("q={long}"),
        "q=".to_owned(),
        "limit=5".to_owned(),
        "q=other&limit=0".to_owned(),
        "q=other&limit=51".to_owned(),
        "q=other&offset=-1".to_owned(),
    ] {
        let answer = search(&fixture, &query).await?;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{query} is refused");
        assert_eq!(answer.code().as_deref(), Some("sites.request.invalid"));
    }
    // 256 times `é`, percent-encoded: 256 Unicode scalar values and 512 bytes.
    let longest = "%C3%A9".repeat(256);
    let answer = search(&fixture, &format!("q={longest}")).await?;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "256 Unicode scalar values are allowed"
    );
    fixture.cleanup().await
}
