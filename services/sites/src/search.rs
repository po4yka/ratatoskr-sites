//! Search over one release (workspace design D8, spec `site-content-delivery`): one row of
//! `sites.release_search` per page, written in the release transaction, and a query that reads only
//! the rows of one release id.
//!
//! The text of a page is every text value of its page tree, the tree that passed the privacy
//! filter, so an index row holds only what the published page shows.

use ratatoskr_sites_contracts::Page;
use ratatoskr_sites_persistence::Database;
use ratatoskr_sites_release::BuiltPage;
use serde_json::Value;
use sqlx::PgConnection;

/// The longest snippet, in Unicode scalar values.
pub const MAX_SNIPPET_CHARS: usize = 300;

/// The most bytes of page text that the `tsvector` of a page reads. `PostgreSQL` refuses a
/// `tsvector` over 1 MiB, and a page body is at most 1 MiB of HTML.
const MAX_INDEXED_BYTES: usize = 256 * 1024;

/// The longest query, in Unicode scalar values.
pub const MAX_QUERY_CHARS: usize = 256;

/// The most results of one answer.
pub const MAX_LIMIT: u32 = 50;

/// One result of a search.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchResult {
    /// The slug of the page.
    pub slug: String,
    /// The title of the page.
    pub title: String,
    /// The start of the text of the page.
    pub snippet: String,
}

/// The text of `page`: every string `value` of its page tree, in document order, with each run of
/// white space as one space.
fn page_text(page: &Page) -> String {
    fn collect(node: &Value, out: &mut String) {
        match node {
            Value::Object(members) => {
                if let Some(Value::String(text)) = members.get("value") {
                    out.push_str(text);
                    out.push(' ');
                }
                if let Some(children) = members.get("children") {
                    collect(children, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }
    let mut raw = String::new();
    if let Ok(tree) = serde_json::to_value(&page.ast) {
        collect(&tree, &mut raw);
    }
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first `max` bytes of `text`, cut at a character boundary.
fn cut_bytes(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// Write the search rows of the pages of `release_id`.
///
/// # Errors
///
/// The query error of a row that cannot be written.
pub(crate) async fn index(
    connection: &mut PgConnection,
    release_id: &str,
    pages: &[BuiltPage],
) -> Result<(), sqlx::Error> {
    for built in pages {
        let text = page_text(&built.page);
        let snippet: String = text.chars().take(MAX_SNIPPET_CHARS).collect();
        sqlx::query(
            "insert into sites.release_search (release_id, slug, title, snippet, document) \
             values ($1, $2, $3, $4, to_tsvector('simple', $3 || ' ' || $5)) \
             on conflict do nothing",
        )
        .bind(release_id)
        .bind(built.page.slug.as_str())
        .bind(built.page.title.as_str())
        .bind(&snippet)
        .bind(cut_bytes(&text, MAX_INDEXED_BYTES))
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

/// The results of `query` in the release `release_id`, in descending relevance with ties by slug,
/// after `offset` results, and whether more follow.
///
/// # Errors
///
/// The query error.
pub async fn search(
    database: &Database,
    release_id: &str,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<(Vec<SearchResult>, bool), sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "select slug, title, snippet from sites.release_search \
         where release_id = $1 and document @@ websearch_to_tsquery('simple', $2) \
         order by ts_rank(document, websearch_to_tsquery('simple', $2)) desc, slug \
         limit $3 offset $4",
    )
    .bind(release_id)
    .bind(query)
    .bind(i64::from(limit) + 1)
    .bind(i64::from(offset))
    .fetch_all(database.pool())
    .await?;
    let has_more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    let results = rows
        .into_iter()
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .map(|(slug, title, snippet)| SearchResult {
            slug,
            title,
            snippet,
        })
        .collect();
    Ok((results, has_more))
}
