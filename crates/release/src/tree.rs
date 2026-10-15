//! The one tree of a note: the parser builds it, the privacy filter changes it, and every renderer
//! and collector reads only the filtered tree (workspace design D5).

use pulldown_cmark::{Alignment, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use ratatoskr_identifiers::BlobRef;
use ratatoskr_sites_contracts::{Anchor, CalloutFold, ImageUrl, LinkUrl, Slug, TableAlign};

/// One node of the tree of a note.
///
/// `Link` and `Image` are what the parser finds. The privacy filter replaces each of them with a
/// resolved node (`PageLink`, `PageEmbed`, `AssetEmbed`, `ExternalLink`, `ExternalImage`), with
/// plain text, or with nothing, so no unresolved reference reaches a renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    Paragraph(Vec<Node>),
    Heading {
        depth: u8,
        children: Vec<Node>,
    },
    ThematicBreak,
    Blockquote(Vec<Node>),
    Callout {
        kind: String,
        fold: Option<CalloutFold>,
        title: Vec<Node>,
        children: Vec<Node>,
    },
    List {
        ordered: bool,
        start: Option<u32>,
        spread: bool,
        children: Vec<Node>,
    },
    ListItem {
        checked: Option<bool>,
        spread: bool,
        children: Vec<Node>,
    },
    Table {
        align: Vec<Option<TableAlign>>,
        children: Vec<Node>,
    },
    TableRow(Vec<Node>),
    TableCell(Vec<Node>),
    Code {
        lang: Option<String>,
        value: String,
    },
    /// Raw HTML of the note. It reaches only the HTML renderer, never the page tree.
    Html(String),
    Text(String),
    Emphasis(Vec<Node>),
    Strong(Vec<Node>),
    Delete(Vec<Node>),
    InlineCode(String),
    Break,
    /// A link as the note writes it: `wiki` is `Some(has_alias)` for `[[…]]`.
    Link {
        url: String,
        wiki: Option<bool>,
        children: Vec<Node>,
    },
    /// An image or embed as the note writes it.
    Image {
        url: String,
        wiki: bool,
        children: Vec<Node>,
    },
    PageLink {
        slug: Slug,
        anchor: Option<Anchor>,
        children: Vec<Node>,
    },
    PageEmbed {
        slug: Slug,
        anchor: Option<Anchor>,
    },
    /// An attachment that the filter resolved: `blob` is the revision of that attachment.
    AssetEmbed {
        blob: BlobRef,
        alt: Option<String>,
    },
    ExternalLink {
        url: LinkUrl,
        title: Option<String>,
        children: Vec<Node>,
    },
    ExternalImage {
        url: ImageUrl,
        alt: Option<String>,
        title: Option<String>,
    },
}

/// A container that the parser has opened and not closed yet.
#[derive(Debug)]
enum Frame {
    Root,
    Paragraph,
    Heading(u8),
    Blockquote,
    List {
        ordered: bool,
        start: Option<u32>,
    },
    Item {
        checked: Option<bool>,
    },
    Table(Vec<Option<TableAlign>>),
    TableHead,
    TableRow,
    TableCell,
    Code(Option<String>),
    HtmlBlock,
    Emphasis,
    Strong,
    Delete,
    Link {
        url: String,
        wiki: Option<bool>,
    },
    Image {
        url: String,
        wiki: bool,
    },
    /// A construct that this builder does not publish; its content stays as plain content.
    Transparent,
}

/// The tree of `body`, a note without frontmatter and comments.
pub(crate) fn parse(body: &str) -> Vec<Node> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_WIKILINKS;
    let mut stack: Vec<(Frame, Vec<Node>)> = vec![(Frame::Root, Vec::new())];
    for event in Parser::new_ext(body, options) {
        match event {
            Event::Start(tag) => stack.push((open(tag), Vec::new())),
            Event::End(TagEnd::TableHead) => close(&mut stack, true),
            Event::End(_) => close(&mut stack, false),
            Event::Text(text) => push(&mut stack, Node::Text(text.into_string())),
            Event::Code(code) => push(&mut stack, Node::InlineCode(code.into_string())),
            Event::Html(html) | Event::InlineHtml(html) => {
                push(&mut stack, Node::Html(html.into_string()));
            }
            Event::SoftBreak => push(&mut stack, Node::Text("\n".to_owned())),
            Event::HardBreak => push(&mut stack, Node::Break),
            Event::Rule => push(&mut stack, Node::ThematicBreak),
            Event::TaskListMarker(checked) => {
                if let Some((Frame::Item { checked: state }, _)) = stack.last_mut() {
                    *state = Some(checked);
                }
            }
            _ => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack, false);
    }
    stack.pop().map(|(_, nodes)| nodes).unwrap_or_default()
}

fn open(tag: Tag<'_>) -> Frame {
    match tag {
        Tag::Paragraph => Frame::Paragraph,
        Tag::Heading { level, .. } => Frame::Heading(level as u8),
        Tag::BlockQuote(_) => Frame::Blockquote,
        Tag::CodeBlock(kind) => Frame::Code(match kind {
            CodeBlockKind::Fenced(info) => info
                .split_whitespace()
                .next()
                .map(str::to_owned)
                .filter(|lang| !lang.is_empty()),
            CodeBlockKind::Indented => None,
        }),
        Tag::HtmlBlock => Frame::HtmlBlock,
        Tag::List(start) => Frame::List {
            ordered: start.is_some(),
            start: start.and_then(|first| u32::try_from(first).ok()),
        },
        Tag::Item => Frame::Item { checked: None },
        Tag::Table(alignments) => Frame::Table(alignments.into_iter().map(align).collect()),
        Tag::TableHead => Frame::TableHead,
        Tag::TableRow => Frame::TableRow,
        Tag::TableCell => Frame::TableCell,
        Tag::Emphasis => Frame::Emphasis,
        Tag::Strong => Frame::Strong,
        Tag::Strikethrough => Frame::Delete,
        Tag::Link {
            link_type,
            dest_url,
            ..
        } => Frame::Link {
            url: match link_type {
                LinkType::Email => format!("mailto:{dest_url}"),
                _ => dest_url.into_string(),
            },
            wiki: match link_type {
                LinkType::WikiLink { has_pothole } => Some(has_pothole),
                _ => None,
            },
        },
        Tag::Image {
            link_type,
            dest_url,
            ..
        } => Frame::Image {
            url: dest_url.into_string(),
            wiki: matches!(link_type, LinkType::WikiLink { .. }),
        },
        _ => Frame::Transparent,
    }
}

const fn align(alignment: Alignment) -> Option<TableAlign> {
    match alignment {
        Alignment::None => None,
        Alignment::Left => Some(TableAlign::Left),
        Alignment::Center => Some(TableAlign::Center),
        Alignment::Right => Some(TableAlign::Right),
    }
}

fn push(stack: &mut [(Frame, Vec<Node>)], node: Node) {
    if let Some((_, children)) = stack.last_mut() {
        children.push(node);
    }
}

/// Close the innermost frame and add its node to the frame around it.
fn close(stack: &mut Vec<(Frame, Vec<Node>)>, head: bool) {
    if stack.len() < 2 {
        return;
    }
    let Some((frame, children)) = stack.pop() else {
        return;
    };
    let nodes = match frame {
        Frame::TableHead if head => vec![Node::TableRow(children)],
        Frame::Root | Frame::Transparent | Frame::HtmlBlock | Frame::TableHead => children,
        frame => vec![node_of(frame, children)],
    };
    if let Some((_, parent)) = stack.last_mut() {
        parent.extend(nodes);
    }
}

fn node_of(frame: Frame, children: Vec<Node>) -> Node {
    match frame {
        Frame::Paragraph => Node::Paragraph(children),
        Frame::Heading(depth) => Node::Heading { depth, children },
        Frame::Blockquote => callout_of(children),
        Frame::List { ordered, start } => {
            let spread = children
                .iter()
                .any(|item| matches!(item, Node::ListItem { spread: true, .. }));
            Node::List {
                ordered,
                start: start.filter(|_| ordered),
                spread,
                children,
            }
        }
        Frame::Item { checked } => item_of(checked, children),
        Frame::Table(align) => Node::Table { align, children },
        Frame::TableRow => Node::TableRow(children),
        Frame::TableCell => Node::TableCell(children),
        Frame::Code(lang) => Node::Code {
            lang,
            value: text_of(&children),
        },
        Frame::Emphasis => Node::Emphasis(children),
        Frame::Strong => Node::Strong(children),
        Frame::Delete => Node::Delete(children),
        Frame::Link { url, wiki } => Node::Link {
            url,
            wiki,
            children,
        },
        Frame::Image { url, wiki } => Node::Image {
            url,
            wiki,
            children,
        },
        Frame::Root | Frame::Transparent | Frame::HtmlBlock | Frame::TableHead => {
            Node::Paragraph(children)
        }
    }
}

/// A list item: the inline content of a tight item becomes one paragraph, as in mdast.
fn item_of(checked: Option<bool>, children: Vec<Node>) -> Node {
    let spread = children
        .iter()
        .any(|node| matches!(node, Node::Paragraph(_)));
    let mut blocks = Vec::new();
    let mut inline = Vec::new();
    for node in children {
        if is_block(&node) {
            if !inline.is_empty() {
                blocks.push(Node::Paragraph(std::mem::take(&mut inline)));
            }
            blocks.push(node);
        } else {
            inline.push(node);
        }
    }
    if !inline.is_empty() {
        blocks.push(Node::Paragraph(inline));
    }
    Node::ListItem {
        checked,
        spread,
        children: blocks,
    }
}

const fn is_block(node: &Node) -> bool {
    matches!(
        node,
        Node::Paragraph(_)
            | Node::Heading { .. }
            | Node::ThematicBreak
            | Node::Blockquote(_)
            | Node::Callout { .. }
            | Node::List { .. }
            | Node::Table { .. }
            | Node::Code { .. }
    )
}

/// A quote whose first line is `[!kind]`, `[!kind]+` or `[!kind]-` with an optional title is an
/// Obsidian callout; any other quote stays a quote.
fn callout_of(children: Vec<Node>) -> Node {
    let mut children = children;
    let Some(Node::Paragraph(first)) = children.first_mut() else {
        return Node::Blockquote(children);
    };
    merge_text(first);
    let Some(Node::Text(text)) = first.first() else {
        return Node::Blockquote(children);
    };
    let Some((kind, fold, rest)) = callout_marker(text) else {
        return Node::Blockquote(children);
    };
    // The title runs to the end of the first line; the rest of the paragraph is content.
    let mut inline = std::mem::take(first);
    inline.remove(0);
    let (title_rest, line_rest) = rest
        .split_once('\n')
        .map_or((rest.as_str(), None), |(a, b)| (a, Some(b)));
    let mut title = Vec::new();
    if !title_rest.trim().is_empty() {
        title.push(Node::Text(title_rest.trim().to_owned()));
    }
    let mut content_inline = Vec::new();
    if let Some(after) = line_rest {
        if !after.is_empty() {
            content_inline.push(Node::Text(after.to_owned()));
        }
        content_inline.extend(inline);
    } else if let Some(at) = inline
        .iter()
        .position(|node| matches!(node, Node::Text(text) if text.contains('\n')))
    {
        let after = inline.split_off(at);
        title.extend(inline);
        content_inline.extend(after);
    } else {
        title.extend(inline);
    }
    let mut blocks = Vec::new();
    trim_leading_break(&mut content_inline);
    if !content_inline.is_empty() {
        blocks.push(Node::Paragraph(content_inline));
    }
    blocks.extend(children.into_iter().skip(1));
    Node::Callout {
        kind,
        fold,
        title,
        children: blocks,
    }
}

fn trim_leading_break(nodes: &mut Vec<Node>) {
    if let Some(Node::Text(text)) = nodes.first_mut() {
        let trimmed = text.trim_start_matches('\n').to_owned();
        if trimmed.is_empty() {
            nodes.remove(0);
        } else {
            *text = trimmed;
        }
    }
}

/// `[!kind]`, an optional `+` or `-`, and the rest of the text.
fn callout_marker(text: &str) -> Option<(String, Option<CalloutFold>, String)> {
    let rest = text.strip_prefix("[!")?;
    let (kind, rest) = rest.split_once(']')?;
    let kind = kind.to_ascii_lowercase();
    if kind.is_empty()
        || kind.len() > 40
        || !kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return None;
    }
    let (fold, rest) = if let Some(rest) = rest.strip_prefix('+') {
        (Some(CalloutFold::Open), rest)
    } else if let Some(rest) = rest.strip_prefix('-') {
        (Some(CalloutFold::Closed), rest)
    } else {
        (None, rest)
    };
    Some((kind, fold, rest.to_owned()))
}

/// Join neighbouring text nodes into one.
pub(crate) fn merge_text(nodes: &mut Vec<Node>) {
    let mut merged: Vec<Node> = Vec::with_capacity(nodes.len());
    for node in nodes.drain(..) {
        match (merged.last_mut(), node) {
            (Some(Node::Text(before)), Node::Text(text)) => before.push_str(&text),
            (_, node) => merged.push(node),
        }
    }
    *nodes = merged;
}

/// The plain text of `nodes`, without markup.
pub(crate) fn text_of(nodes: &[Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(value) | Node::InlineCode(value) | Node::Code { value, .. } => {
                text.push_str(value);
            }
            Node::Break => text.push('\n'),
            Node::Paragraph(children)
            | Node::Blockquote(children)
            | Node::TableRow(children)
            | Node::TableCell(children)
            | Node::Emphasis(children)
            | Node::Strong(children)
            | Node::Delete(children)
            | Node::Heading { children, .. }
            | Node::List { children, .. }
            | Node::ListItem { children, .. }
            | Node::Table { children, .. }
            | Node::Link { children, .. }
            | Node::Image { children, .. }
            | Node::PageLink { children, .. }
            | Node::ExternalLink { children, .. } => text.push_str(&text_of(children)),
            Node::Callout {
                title, children, ..
            } => {
                text.push_str(&text_of(title));
                text.push_str(&text_of(children));
            }
            Node::ThematicBreak
            | Node::Html(_)
            | Node::PageEmbed { .. }
            | Node::AssetEmbed { .. }
            | Node::ExternalImage { .. } => {}
        }
    }
    text
}
