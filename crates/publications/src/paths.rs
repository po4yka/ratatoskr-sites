//! Vault paths and folders compare by the path key of `ratatoskr-notes`: the NFC path folded with
//! Unicode default case folding. So the excluded folder `Private` excludes `private/x.md`, and a
//! folder name in NFD matches a path in NFC, as in notes.

use unicode_normalization::UnicodeNormalization as _;

/// The path key of `path`.
#[must_use]
pub fn path_key(path: &str) -> String {
    let nfc: String = path.nfc().collect();
    caseless::default_case_fold_str(&nfc)
}

/// Whether `path` is in `folder` or in one of its subfolders, by path key. A leading or trailing
/// `/` of the folder is ignored, and an empty folder matches nothing.
#[must_use]
pub fn in_folder(path: &str, folder: &str) -> bool {
    let folder = path_key(folder.trim_matches('/'));
    !folder.is_empty()
        && path_key(path)
            .strip_prefix(&folder)
            .is_some_and(|rest| rest.starts_with('/'))
}
