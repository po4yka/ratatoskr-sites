//! SITE-025 task 5.12: every page has one stable slug (workspace spec `site-publications`,
//! requirement "Every page has one stable slug"), and the proposed class `sites.slug_invalid`.

use ratatoskr_sites_publications::test_support::revision_of;
use ratatoskr_sites_publications::{IndexedFile, assign_slugs};
use serde_json::{Map, Value, json};

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn note(path: &str, frontmatter: Value) -> Result<IndexedFile, Box<dyn std::error::Error>> {
    let frontmatter: Option<Map<String, Value>> = match frontmatter {
        Value::Object(map) => Some(map),
        _ => None,
    };
    Ok(IndexedFile {
        path: path.to_owned(),
        revision: revision_of(path.as_bytes(), "text/markdown")?,
        frontmatter,
        tags: Vec::new(),
        headings: Vec::new(),
        blocks: Vec::new(),
        links: Vec::new(),
        embeds: Vec::new(),
    })
}

fn slugs_of(files: &[IndexedFile]) -> Result<Vec<(String, String)>, String> {
    let selected: Vec<&IndexedFile> = files.iter().collect();
    assign_slugs(&selected)
        .map(|slugs| {
            slugs
                .into_iter()
                .map(|(slug, path)| (path, slug.as_str().to_owned()))
                .collect()
        })
        .map_err(|error| format!("{}: {}", error.class, error.detail))
}

#[test]
fn slug_from_path_is_lower_case_and_dashed() -> Outcome {
    let files = [
        note("Notes/A B.md", Value::Null)?,
        note("Blog/Café au lait!.md", Value::Null)?,
        note("Daily/2024-01-01.md", Value::Null)?,
    ];
    let mut slugs = slugs_of(&files)?;
    slugs.sort();
    assert_eq!(
        slugs,
        [
            (
                "Blog/Café au lait!.md".to_owned(),
                "blog/caf-au-lait-".to_owned()
            ),
            (
                "Daily/2024-01-01.md".to_owned(),
                "daily/2024-01-01".to_owned()
            ),
            ("Notes/A B.md".to_owned(), "notes/a-b".to_owned()),
        ]
    );
    Ok(())
}

#[test]
fn frontmatter_slug_wins() -> Outcome {
    let files = [note(
        "Notes/Long Title.md",
        json!({ "slug": "handbook/start" }),
    )?];
    assert_eq!(
        slugs_of(&files)?,
        [(
            "Notes/Long Title.md".to_owned(),
            "handbook/start".to_owned()
        )]
    );
    Ok(())
}

#[test]
fn colliding_paths_fail_the_build() -> Outcome {
    let files = [
        note("Notes/A B.md", Value::Null)?,
        note("notes/a-b.md", Value::Null)?,
    ];
    let selected: Vec<&IndexedFile> = files.iter().collect();
    let built = assign_slugs(&selected);
    let error = built.as_ref().err();
    assert_eq!(
        error.map(|error| error.class),
        Some("sites.slug_collision"),
        "two selected files with one slug make no release: {built:?}"
    );
    let detail = error.map_or("", |error| error.detail.as_str());
    assert!(
        detail.contains("Notes/A B.md") && detail.contains("notes/a-b.md"),
        "the operation detail names both files: {detail}"
    );
    Ok(())
}

#[test]
fn frontmatter_slug_outside_the_grammar_fails() -> Outcome {
    let files = [note("Notes/Bad.md", json!({ "slug": "Not A Slug" }))?];
    let selected: Vec<&IndexedFile> = files.iter().collect();
    let built = assign_slugs(&selected);
    let error = built.as_ref().err();
    assert_eq!(
        error.map(|error| error.class),
        Some("sites.slug_invalid"),
        "a frontmatter slug outside the grammar makes no release: {built:?}"
    );
    assert!(
        error.is_some_and(|error| error.detail.contains("Notes/Bad.md")),
        "the detail names the file: {built:?}"
    );
    Ok(())
}
