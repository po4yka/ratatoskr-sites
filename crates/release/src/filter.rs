//! The privacy filter: one pass on the tree of a selected note, before any renderer (workspace
//! design D5, spec `site-release-privacy`).
//!
//! - A link to a selected note becomes a page link; a link to anything else is the text that the
//!   author wrote, exactly as for a link whose target does not exist.
//! - An embed of a selected note or of a published attachment stays; any other embed is removed
//!   without a trace.
//! - The target of a link comes only from the notes index of the same file (workspace design D3),
//!   matched by the text that the note writes. Nothing is read from the target.
//!
//! Comments were removed from the text before the parse, so a link inside a comment is not in the
//! tree and makes no edge, also when the index lists it.

use std::cell::RefCell;
use std::collections::BTreeMap;

use ratatoskr_identifiers::BlobRef;
use ratatoskr_sites_contracts::{Anchor, ImageUrl, LinkUrl, PageTitle, Slug};
use ratatoskr_sites_publications::{IndexedFile, IndexedLink};

use crate::tree::{Node, merge_text, text_of};

/// What the filter of one note may name.
#[derive(Debug)]
pub(crate) struct Scope<'a> {
    /// The note.
    pub(crate) file: &'a IndexedFile,
    /// The selected notes of the same vault connection: vault path to slug and title.
    pub(crate) pages: &'a BTreeMap<String, (Slug, PageTitle)>,
    /// The attachments that the publication may publish: vault path to blob.
    pub(crate) attachments: &'a BTreeMap<String, BlobRef>,
    /// The index entries that the filter resolved to a page or an asset, `true` for an embed, with
    /// no display text: what a later build from the release needs, and nothing of a comment.
    pub(crate) resolved: RefCell<Vec<(bool, IndexedLink)>>,
}

/// What a reference names after the filter.
enum Target<'a> {
    Page(Slug, Option<Anchor>, PageTitle),
    Asset(&'a BlobRef),
    Nothing,
}

/// The filtered tree of `nodes`.
pub(crate) fn filter(nodes: Vec<Node>, scope: &Scope<'_>) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        out.extend(filter_node(node, scope));
    }
    merge_text(&mut out);
    out
}

fn filter_node(node: Node, scope: &Scope<'_>) -> Vec<Node> {
    match node {
        Node::Link {
            url,
            title,
            wiki,
            children,
        } => link(&url, &title, wiki, filter(children, scope), scope),
        Node::Image {
            url,
            title,
            wiki,
            children,
        } => embed(&url, &title, wiki, &children, scope)
            .into_iter()
            .collect(),
        Node::Paragraph(children) => {
            let mut children = filter(children, scope);
            strip_block_id(&mut children);
            if is_blank(&children) {
                Vec::new()
            } else {
                vec![Node::Paragraph(children)]
            }
        }
        node => vec![filter_children(node, scope)],
    }
}

/// `node` with its children filtered.
fn filter_children(node: Node, scope: &Scope<'_>) -> Node {
    match node {
        Node::Heading { depth, children } => Node::Heading {
            depth,
            children: filter(children, scope),
        },
        Node::Blockquote(children) => Node::Blockquote(filter(children, scope)),
        Node::Callout {
            kind,
            fold,
            title,
            children,
        } => Node::Callout {
            kind,
            fold,
            title: filter(title, scope),
            children: filter(children, scope),
        },
        Node::List {
            ordered,
            start,
            spread,
            children,
        } => Node::List {
            ordered,
            start,
            spread,
            children: filter(children, scope),
        },
        Node::ListItem {
            checked,
            spread,
            children,
        } => Node::ListItem {
            checked,
            spread,
            children: filter(children, scope),
        },
        Node::Table { align, children } => Node::Table {
            align,
            children: filter(children, scope),
        },
        Node::TableRow(children) => Node::TableRow(filter(children, scope)),
        Node::TableCell(children) => Node::TableCell(filter(children, scope)),
        Node::Emphasis(children) => Node::Emphasis(filter(children, scope)),
        Node::Strong(children) => Node::Strong(filter(children, scope)),
        Node::Delete(children) => Node::Delete(filter(children, scope)),
        node => node,
    }
}

/// A link: to a page, to a published attachment (as its embed, workspace-local design 7), to an
/// external URL, or else the author's text.
fn link(
    url: &str,
    title: &str,
    wiki: Option<bool>,
    children: Vec<Node>,
    scope: &Scope<'_>,
) -> Vec<Node> {
    if wiki.is_none() && is_external(url) {
        return match LinkUrl::parse(url) {
            Ok(url) => vec![Node::ExternalLink {
                url,
                title: (!title.is_empty()).then(|| title.to_owned()),
                children,
            }],
            Err(_) => children,
        };
    }
    match resolve(&scope.file.links, url, scope, false) {
        Target::Page(slug, anchor, _) => vec![Node::PageLink {
            slug,
            anchor,
            children,
        }],
        Target::Asset(blob) => vec![asset(blob, None)],
        Target::Nothing => children,
    }
}

/// An embed or an image: of a page, of a published attachment, of an external image, or nothing.
fn embed(url: &str, title: &str, wiki: bool, children: &[Node], scope: &Scope<'_>) -> Option<Node> {
    // The text of a wiki embed is its target or a size, never alternative text.
    let alt = (!wiki)
        .then(|| text_of(children))
        .filter(|alt| !alt.is_empty());
    if !wiki && is_external(url) {
        return ImageUrl::parse(url).ok().map(|url| Node::ExternalImage {
            url,
            alt,
            title: (!title.is_empty()).then(|| title.to_owned()),
        });
    }
    match resolve(&scope.file.embeds, url, scope, true) {
        Target::Page(slug, anchor, title) => Some(Node::PageEmbed {
            slug,
            anchor,
            title,
        }),
        Target::Asset(blob) => Some(asset(blob, alt)),
        Target::Nothing => None,
    }
}

fn asset(blob: &BlobRef, alt: Option<String>) -> Node {
    Node::AssetEmbed {
        blob: blob.clone(),
        alt,
    }
}

/// The target of the reference `written` that the notes index of the file lists.
fn resolve<'a>(
    listed: &[IndexedLink],
    written: &str,
    scope: &Scope<'a>,
    embed: bool,
) -> Target<'a> {
    let decoded = percent_decoded(written);
    let Some(entry) = listed
        .iter()
        .find(|entry| entry.link == written || entry.link == decoded)
    else {
        return Target::Nothing;
    };
    let Some(path) = entry.target.as_deref() else {
        return Target::Nothing;
    };
    let target = if let Some((slug, title)) = scope.pages.get(path) {
        Target::Page(slug.clone(), anchor_of(&decoded), title.clone())
    } else {
        match scope.attachments.get(path) {
            Some(blob) => Target::Asset(blob),
            None => return Target::Nothing,
        }
    };
    let kept = IndexedLink {
        display_text: String::new(),
        ..entry.clone()
    };
    let mut resolved = scope.resolved.borrow_mut();
    if !resolved.contains(&(embed, kept.clone())) {
        resolved.push((embed, kept));
    }
    target
}

/// The anchor of the subpath of a reference: `#^id` is the block `^id`, `#Heading` the heading.
fn anchor_of(reference: &str) -> Option<Anchor> {
    let (_, subpath) = reference.split_once('#')?;
    let subpath = subpath.split('#').next_back().unwrap_or(subpath);
    if subpath.starts_with('^') {
        Anchor::parse(subpath).ok()
    } else {
        heading_anchor(subpath)
    }
}

/// The anchor of a heading with the text `text`, by the one rule of [`crate::ids::heading_id`].
pub(crate) fn heading_anchor(text: &str) -> Option<Anchor> {
    crate::ids::heading_id(text).and_then(|id| Anchor::parse(&id).ok())
}

fn is_external(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// `text` with each `%XX` replaced by its byte, when the result is UTF-8.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let (b'%', Some(decoded)) = (byte, hex) {
            out.push(decoded);
            at += 3;
        } else {
            out.push(byte);
            at += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// Remove a trailing block id ` ^id` from the end of a paragraph: Obsidian does not show it.
fn strip_block_id(children: &mut [Node]) {
    let Some(Node::Text(text)) = children.last_mut() else {
        return;
    };
    let trimmed = text.trim_end();
    if let Some(at) = trimmed.rfind('^') {
        let id = trimmed.get(at + 1..).unwrap_or("");
        let before = trimmed.get(..at).unwrap_or("");
        if !id.is_empty()
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && (before.is_empty() || before.ends_with(char::is_whitespace))
        {
            *text = before.trim_end().to_owned();
        }
    }
}

fn is_blank(nodes: &[Node]) -> bool {
    nodes.iter().all(|node| match node {
        Node::Text(text) => text.trim().is_empty(),
        Node::Break => true,
        _ => false,
    })
}
