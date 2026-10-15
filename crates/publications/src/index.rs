//! The index of one vault connection, as the notes internal index route
//! `GET /internal/connections/{connection}/files` gives it (workspace change
//! `add-site-publications`, task 4.1): per file, the current revision, the frontmatter, the tags,
//! the headings, the block ids and the resolved outgoing links and embeds.
//!
//! The route is not part of a contract crate. This is the shape that the sites service reads; the
//! field names follow `ratatoskr-notes-index`.

use ratatoskr_identifiers::BlobRef;
use serde_json::{Map, Value};

/// One file of a vault connection.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedFile {
    /// The vault path in NFC, with `/` between folders, for example `blog/hello.md`.
    pub path: String,
    /// The bytes of the current revision in the notes blob root.
    pub revision: BlobRef,
    /// The frontmatter as JSON, or `None` when the note has none or the file is not Markdown.
    #[serde(default)]
    pub frontmatter: Option<Map<String, Value>>,
    /// The tags of the note, each with its `#`, in the order of the note.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The headings, in the order of the note.
    #[serde(default)]
    pub headings: Vec<IndexedHeading>,
    /// The block ids, without the `^`, in the order of the note.
    #[serde(default)]
    pub blocks: Vec<String>,
    /// The links of the text, in the order of the note.
    #[serde(default)]
    pub links: Vec<IndexedLink>,
    /// The embeds of the text, in the order of the note.
    #[serde(default)]
    pub embeds: Vec<IndexedLink>,
}

impl IndexedFile {
    /// Whether the file is a Markdown note.
    #[must_use]
    pub fn is_markdown(&self) -> bool {
        std::path::Path::new(&self.path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    }

    /// The value of the frontmatter key `key`.
    #[must_use]
    pub fn frontmatter_value(&self, key: &str) -> Option<&Value> {
        self.frontmatter.as_ref()?.get(key)
    }
}

/// One heading of a note.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedHeading {
    /// 1 to 6.
    pub level: u8,
    /// The text of the heading as the note writes it.
    pub heading: String,
}

/// How a link is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkForm {
    /// `[[target]]`, `[[target|alias]]`, `![[target]]`.
    Wiki,
    /// `[text](target)`, `![text](target)`.
    Markdown,
}

/// One link or embed of a note, with the file that the notes resolver found for it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedLink {
    /// The target with its subpath, in NFC, as the note writes it: `Beta#Section One`.
    pub link: String,
    /// The text that Obsidian shows for the link. Not stored with the sources of a release, which
    /// rendering does not read it from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_text: String,
    /// How the link is written.
    pub form: LinkForm,
    /// The vault path of the target, or `None` when the link is unresolved.
    #[serde(default)]
    pub target: Option<String>,
}
