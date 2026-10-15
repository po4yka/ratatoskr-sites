//! SITE-025 task 5.10: the selection precedence of the workspace spec `site-publications`.
//!
//! The fixture vault `fixtures/selection-vault/` is one vault connection. Two sites read it through
//! two publications: `site-a` (publication `pub-a`) and `site-b` (publication `pub-b`). Each test
//! asserts the exact set of paths that a publication selects, and names the file of its case.

use std::collections::BTreeSet;
use std::path::Path;

use ratatoskr_sites_contracts::{PublicationId, SiteId};
use ratatoskr_sites_publications::test_support::load_index;
use ratatoskr_sites_publications::{IndexedFile, Publication, PublicationMode, select};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/selection-vault"
);

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn files() -> Result<Vec<IndexedFile>, Box<dyn std::error::Error>> {
    Ok(load_index(Path::new(FIXTURE))?)
}

fn publication(
    site: &str,
    id: &str,
    mode: PublicationMode,
    included: &[&str],
    excluded: &[&str],
    tags: &[&str],
) -> Result<Publication, Box<dyn std::error::Error>> {
    Ok(Publication {
        site_id: SiteId::parse(site)?,
        publication_id: PublicationId::parse(id)?,
        mode,
        included_folders: included.iter().map(|folder| (*folder).to_owned()).collect(),
        excluded_folders: excluded.iter().map(|folder| (*folder).to_owned()).collect(),
        included_tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
    })
}

/// `site-a`: flagged, reads `blog/` and `private/`, never `private/`, no tag.
fn pub_a() -> Result<Publication, Box<dyn std::error::Error>> {
    publication(
        "site-a",
        "pub-a",
        PublicationMode::Flagged,
        &["blog", "private"],
        &["private"],
        &[],
    )
}

/// `site-b`: flagged, reads `blog/`, selects the tag `public`.
fn pub_b() -> Result<Publication, Box<dyn std::error::Error>> {
    publication(
        "site-b",
        "pub-b",
        PublicationMode::Flagged,
        &["blog"],
        &[],
        &["public"],
    )
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

fn expect_a() -> BTreeSet<String> {
    set(&["blog/both.md", "blog/hello.md"])
}

fn expect_b() -> BTreeSet<String> {
    set(&["blog/both.md", "blog/tagged.md"])
}

#[test]
fn publish_list_limits_a_file_to_named_sites() -> Outcome {
    let files = files()?;
    // `blog/hello.md` has `publish: [site-a]`: site-a takes it, site-b does not.
    assert_eq!(
        select(&pub_a()?, &files),
        expect_a(),
        "site-a takes blog/hello.md"
    );
    assert_eq!(
        select(&pub_b()?, &files),
        expect_b(),
        "site-b does not take blog/hello.md"
    );
    Ok(())
}

#[test]
fn excluded_folder_wins_over_publish_true() -> Outcome {
    // `private/launch.md` has `publish: true` inside an included and excluded folder.
    assert_eq!(
        select(&pub_a()?, &files()?),
        expect_a(),
        "private/launch.md is not selected"
    );
    Ok(())
}

#[test]
fn publish_true_outside_included_folders_is_not_selected() -> Outcome {
    let files = files()?;
    // `journal/today.md` has `publish: true` outside the included folders of both.
    assert_eq!(
        select(&pub_a()?, &files),
        expect_a(),
        "journal/today.md is not in site-a"
    );
    assert_eq!(
        select(&pub_b()?, &files),
        expect_b(),
        "journal/today.md is not in site-b"
    );
    Ok(())
}

#[test]
fn publish_false_overrides_included_tag() -> Outcome {
    // `blog/tagged-hidden.md` has the included tag `#public` and `publish: false`;
    // `blog/tagged.md` has the tag alone.
    assert_eq!(
        select(&pub_b()?, &files()?),
        expect_b(),
        "blog/tagged-hidden.md is not selected, blog/tagged.md is"
    );
    Ok(())
}

#[test]
fn default_is_not_selected() -> Outcome {
    let files = files()?;
    // `blog/plain.md` has no publish key and no included tag.
    assert_eq!(
        select(&pub_a()?, &files),
        expect_a(),
        "blog/plain.md is not in site-a"
    );
    assert_eq!(
        select(&pub_b()?, &files),
        expect_b(),
        "blog/plain.md is not in site-b"
    );
    Ok(())
}

#[test]
fn attachment_in_included_folder_needs_a_published_use() -> Outcome {
    let files = files()?;
    // `blog/draft-cover.png` is in an included folder, also for a publication in the mode `all`.
    let all = publication(
        "site-a",
        "pub-all",
        PublicationMode::All,
        &["blog"],
        &[],
        &[],
    )?;
    assert_eq!(
        select(&all, &files),
        set(&[
            "blog/both.md",
            "blog/hello.md",
            "blog/plain.md",
            "blog/tagged.md"
        ]),
        "blog/draft-cover.png is not selected by a folder"
    );
    assert_eq!(select(&pub_b()?, &files), expect_b(), "nor by site-b");
    Ok(())
}

#[test]
fn obsidian_folder_is_never_selected() -> Outcome {
    // A publication that includes `.obsidian` by mistake, in the mode `all`.
    let mistaken = publication(
        "site-a",
        "pub-mistaken",
        PublicationMode::All,
        &[".obsidian", "blog"],
        &[],
        &[],
    )?;
    assert_eq!(
        select(&mistaken, &files()?),
        set(&[
            "blog/both.md",
            "blog/hello.md",
            "blog/plain.md",
            "blog/tagged.md"
        ]),
        "nothing under .obsidian/ is selected"
    );
    Ok(())
}
