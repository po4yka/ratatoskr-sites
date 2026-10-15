//! A build that cannot make a release.

/// Why a build makes no release: a stable class, and a detail for the owner's operation.
///
/// The detail may name the vault paths of selected files; it never names an unselected file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the build failed with {class}: {detail}")]
pub struct BuildError {
    /// The stable class, for example `sites.slug_collision`.
    pub class: &'static str,
    /// What the owner reads in the operation.
    pub detail: String,
}
