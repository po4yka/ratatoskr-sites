//! The slug of each selected page (workspace spec `site-publications`, requirement "Every page has
//! one stable slug").

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use ratatoskr_sites_contracts::Slug;
use serde_json::Value;

use crate::error::BuildError;
use crate::index::IndexedFile;

/// The class of two selected files with one slug.
pub const SLUG_COLLISION: &str = "sites.slug_collision";

/// The class of a slug outside the grammar of [`Slug`]. The workspace spec names no class for it;
/// this one is proposed by this repository.
pub const SLUG_INVALID: &str = "sites.slug_invalid";

/// The slug of each file of `selected`, by slug, with the vault path of its file.
///
/// # Errors
///
/// [`BuildError`] with [`SLUG_INVALID`] when a frontmatter `slug` or a derived slug is outside the
/// grammar, and with [`SLUG_COLLISION`] when two files get one slug. The detail names the files.
pub fn assign_slugs(selected: &[&IndexedFile]) -> Result<BTreeMap<Slug, String>, BuildError> {
    let mut slugs: BTreeMap<Slug, String> = BTreeMap::new();
    for file in selected {
        let slug = slug_of(file)?;
        match slugs.entry(slug) {
            Entry::Vacant(vacant) => {
                vacant.insert(file.path.clone());
            }
            Entry::Occupied(occupied) => {
                return Err(BuildError {
                    class: SLUG_COLLISION,
                    detail: format!(
                        "{} and {} have the slug {}",
                        occupied.get(),
                        file.path,
                        occupied.key().as_str()
                    ),
                });
            }
        }
    }
    Ok(slugs)
}

/// The frontmatter `slug` of `file`, or else its vault path without `.md`, lower-cased, with each
/// run of other characters than ASCII letters, digits and `/` replaced by one `-`.
fn slug_of(file: &IndexedFile) -> Result<Slug, BuildError> {
    let invalid = || BuildError {
        class: SLUG_INVALID,
        detail: format!("{} has no valid slug", file.path),
    };
    let text = match file.frontmatter_value("slug") {
        Some(Value::String(slug)) => slug.clone(),
        Some(_) => return Err(invalid()),
        None => derived(&file.path),
    };
    Slug::parse(&text).map_err(|_| invalid())
}

fn derived(path: &str) -> String {
    let stem = path
        .strip_suffix(".md")
        .or_else(|| path.strip_suffix(".MD"))
        .unwrap_or(path);
    let mut slug = String::with_capacity(stem.len());
    let mut in_run = false;
    for character in stem.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() || character == '/' {
            slug.push(character);
            in_run = false;
        } else if !in_run {
            slug.push('-');
            in_run = true;
        }
    }
    slug
}
