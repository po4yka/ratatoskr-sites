//! The slug of each selected page.

use std::collections::BTreeMap;

use ratatoskr_sites_contracts::Slug;

use crate::error::BuildError;
use crate::index::IndexedFile;

/// The slug of each file of `selected`, by slug, with the vault path of its file.
///
/// # Errors
///
/// [`BuildError`] when a slug cannot be made.
pub fn assign_slugs(selected: &[&IndexedFile]) -> Result<BTreeMap<Slug, String>, BuildError> {
    let mut slugs = BTreeMap::new();
    for file in selected {
        let slug = Slug::parse("page").map_err(|_| BuildError {
            class: "sites.slug_invalid",
            detail: file.path.clone(),
        })?;
        slugs.insert(slug, file.path.clone());
    }
    Ok(slugs)
}
