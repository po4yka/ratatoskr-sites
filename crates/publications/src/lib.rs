//! The publications of a site: the notes index that a publication reads, and which files of one
//! vault connection a publication selects (workspace spec `site-publications`).
//!
//! This crate reads no file and no database. It decides from the index of a vault connection, as
//! the notes internal index route gives it, and nothing else.

pub mod error;
pub mod index;
pub mod paths;
pub mod selection;
pub mod slug;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use crate::error::BuildError;
pub use crate::index::{IndexedFile, IndexedHeading, IndexedLink, LinkForm};
pub use crate::paths::{in_folder, path_key};
pub use crate::selection::{Publication, PublicationMode, select, tag_key, tag_matches};
pub use crate::slug::assign_slugs;
