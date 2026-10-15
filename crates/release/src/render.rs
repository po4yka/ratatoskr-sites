//! The two renderers of a filtered tree: the page tree (`sites/page.v1` `ast`) and the HTML body.
//! Both read only the filtered tree (workspace design D2 and D5).

use std::fmt::Write as _;

use ratatoskr_sites_contracts::{
    Anchor, CalloutFold, CalloutKind, EmbedTarget, PageNode, TableAlign,
};

use crate::filter::heading_anchor;
use crate::tree::{Node, text_of};

/// The page tree of `nodes`. Raw HTML has no node in the page tree.
pub(crate) fn to_ast(nodes: &[Node]) -> Vec<PageNode> {
    nodes.iter().filter_map(ast_node).collect()
}

fn ast_node(node: &Node) -> Option<PageNode> {
    Some(match node {
        Node::Paragraph(children) => PageNode::Paragraph {
            children: to_ast(children),
        },
        Node::Heading { depth, children } => PageNode::Heading {
            depth: *depth,
            anchor: anchor_of_heading(children)?,
            children: to_ast(children),
        },
        Node::ThematicBreak => PageNode::ThematicBreak {},
        Node::Blockquote(children) => PageNode::Blockquote {
            children: to_ast(children),
        },
        Node::Callout {
            kind,
            fold,
            title,
            children,
        } => PageNode::Callout {
            kind: CalloutKind::parse(kind).ok()?,
            fold: *fold,
            title: to_ast(title),
            children: to_ast(children),
        },
        Node::List {
            ordered,
            start,
            spread,
            children,
        } => PageNode::List {
            ordered: *ordered,
            start: *start,
            spread: *spread,
            children: to_ast(children),
        },
        Node::ListItem {
            checked,
            spread,
            children,
        } => PageNode::ListItem {
            checked: *checked,
            spread: *spread,
            children: to_ast(children),
        },
        Node::Table { align, children } => PageNode::Table {
            align: align.clone(),
            children: to_ast(children),
        },
        Node::TableRow(children) => PageNode::TableRow {
            children: to_ast(children),
        },
        Node::TableCell(children) => PageNode::TableCell {
            children: to_ast(children),
        },
        Node::Code { lang, value } => PageNode::Code {
            lang: lang.clone(),
            meta: None,
            value: value.clone(),
        },
        Node::Text(value) if value.is_empty() => return None,
        Node::Text(value) => PageNode::Text {
            value: value.clone(),
        },
        Node::Emphasis(children) => PageNode::Emphasis {
            children: to_ast(children),
        },
        Node::Strong(children) => PageNode::Strong {
            children: to_ast(children),
        },
        Node::Delete(children) => PageNode::Delete {
            children: to_ast(children),
        },
        Node::InlineCode(value) => PageNode::InlineCode {
            value: value.clone(),
        },
        Node::Break => PageNode::Break {},
        _ => return reference_node(node),
    })
}

fn reference_node(node: &Node) -> Option<PageNode> {
    Some(match node {
        Node::PageLink {
            slug,
            anchor,
            children,
        } => PageNode::WikiLink {
            slug: slug.clone(),
            anchor: anchor.clone(),
            children: to_ast(children),
        },
        Node::PageEmbed { slug, anchor } => PageNode::Embed {
            target: EmbedTarget::Page {
                slug: slug.clone(),
                anchor: anchor.clone(),
            },
        },
        Node::AssetEmbed {
            sha256,
            media_type,
            alt,
        } => PageNode::Embed {
            target: EmbedTarget::Asset {
                sha256: sha256.clone(),
                media_type: media_type.clone(),
                alt: alt.clone(),
            },
        },
        Node::ExternalLink {
            url,
            title,
            children,
        } => PageNode::Link {
            url: url.clone(),
            title: title.clone(),
            children: to_ast(children),
        },
        Node::ExternalImage { url, alt, title } => PageNode::Image {
            url: url.clone(),
            alt: alt.clone(),
            title: title.clone(),
        },
        // Raw HTML and unresolved references never reach the page tree.
        _ => return None,
    })
}

/// The anchor of a heading; a heading without a usable text gets `section`, which matches the
/// pattern of [`Anchor`].
fn anchor_of_heading(children: &[Node]) -> Option<Anchor> {
    heading_anchor(&text_of(children)).or_else(|| Anchor::parse("section").ok())
}

/// The HTML body of `nodes`. Raw HTML of the note is written as it is; the HTML allowlist runs on
/// the whole body after this.
pub(crate) fn to_html(nodes: &[Node]) -> String {
    let mut html = String::new();
    for node in nodes {
        write_node(&mut html, node);
    }
    html
}

fn wrap(html: &mut String, tag: &str, children: &[Node]) {
    let _ = write!(html, "<{tag}>");
    html.push_str(&to_html(children));
    let _ = write!(html, "</{tag}>");
}

fn write_node(html: &mut String, node: &Node) {
    match node {
        Node::Paragraph(children) => wrap(html, "p", children),
        Node::Heading { depth, children } => {
            match anchor_of_heading(children) {
                Some(anchor) => {
                    let _ = write!(html, "<h{depth} id=\"{}\">", escape(anchor.as_str()));
                }
                None => {
                    let _ = write!(html, "<h{depth}>");
                }
            }
            html.push_str(&to_html(children));
            let _ = write!(html, "</h{depth}>");
        }
        Node::ThematicBreak => html.push_str("<hr>"),
        Node::Blockquote(children) => wrap(html, "blockquote", children),
        Node::Callout {
            kind,
            fold,
            title,
            children,
        } => {
            let fold = match fold {
                Some(CalloutFold::Open) => " data-fold=\"open\"",
                Some(CalloutFold::Closed) => " data-fold=\"closed\"",
                None => "",
            };
            let _ = write!(
                html,
                "<div class=\"callout\" data-callout=\"{}\"{fold}><div class=\"callout-title\">",
                escape(kind)
            );
            html.push_str(&to_html(title));
            html.push_str("</div><div class=\"callout-content\">");
            html.push_str(&to_html(children));
            html.push_str("</div></div>");
        }
        Node::List {
            ordered,
            start,
            children,
            ..
        } => match (ordered, start) {
            (true, Some(first)) if *first != 1 => {
                let _ = write!(html, "<ol start=\"{first}\">");
                html.push_str(&to_html(children));
                html.push_str("</ol>");
            }
            (true, _) => wrap(html, "ol", children),
            (false, _) => wrap(html, "ul", children),
        },
        Node::ListItem {
            checked, children, ..
        } => {
            html.push_str("<li>");
            match checked {
                Some(true) => html.push_str("<input type=\"checkbox\" checked disabled> "),
                Some(false) => html.push_str("<input type=\"checkbox\" disabled> "),
                None => {}
            }
            html.push_str(&to_html(children));
            html.push_str("</li>");
        }
        _ => write_table_or_inline(html, node),
    }
}

fn write_table_or_inline(html: &mut String, node: &Node) {
    match node {
        Node::Table { align, children } => {
            html.push_str("<table>");
            for (index, row) in children.iter().enumerate() {
                let cell = if index == 0 { "th" } else { "td" };
                write_row(html, row, align, cell);
            }
            html.push_str("</table>");
        }
        Node::TableRow(children) | Node::TableCell(children) => {
            html.push_str(&to_html(children));
        }
        Node::Code { lang, value } => {
            match lang {
                Some(lang) => {
                    let _ = write!(html, "<pre><code class=\"language-{}\">", escape(lang));
                }
                None => html.push_str("<pre><code>"),
            }
            html.push_str(&escape(value));
            html.push_str("</code></pre>");
        }
        Node::Html(raw) => html.push_str(raw),
        Node::Text(value) => html.push_str(&escape(value)),
        Node::Emphasis(children) => wrap(html, "em", children),
        Node::Strong(children) => wrap(html, "strong", children),
        Node::Delete(children) => wrap(html, "del", children),
        Node::InlineCode(value) => {
            let _ = write!(html, "<code>{}</code>", escape(value));
        }
        Node::Break => html.push_str("<br>"),
        _ => write_reference(html, node),
    }
}

fn write_row(html: &mut String, row: &Node, align: &[Option<TableAlign>], cell: &str) {
    let Node::TableRow(cells) = row else {
        return;
    };
    html.push_str("<tr>");
    for (column, content) in cells.iter().enumerate() {
        let style = match align.get(column).copied().flatten() {
            Some(TableAlign::Left) => " align=\"left\"",
            Some(TableAlign::Center) => " align=\"center\"",
            Some(TableAlign::Right) => " align=\"right\"",
            None => "",
        };
        let _ = write!(html, "<{cell}{style}>");
        if let Node::TableCell(children) = content {
            html.push_str(&to_html(children));
        }
        let _ = write!(html, "</{cell}>");
    }
    html.push_str("</tr>");
}

fn write_reference(html: &mut String, node: &Node) {
    match node {
        Node::PageLink {
            slug,
            anchor,
            children,
        } => {
            let _ = write!(
                html,
                "<a href=\"{}\">",
                escape(&page_href(slug.as_str(), anchor.as_ref()))
            );
            html.push_str(&to_html(children));
            html.push_str("</a>");
        }
        Node::PageEmbed { slug, anchor } => {
            let href = page_href(slug.as_str(), anchor.as_ref());
            let _ = write!(
                html,
                "<a class=\"embed\" href=\"{}\">{}</a>",
                escape(&href),
                escape(slug.as_str())
            );
        }
        Node::AssetEmbed {
            sha256,
            media_type,
            alt,
        } => {
            let src = format!("assets/{}", sha256.as_str());
            if media_type.as_str().starts_with("image/") {
                let alt = alt.as_deref().unwrap_or("");
                let _ = write!(html, "<img src=\"{src}\" alt=\"{}\">", escape(alt));
            } else {
                let text = alt.as_deref().unwrap_or(sha256.as_str());
                let _ = write!(html, "<a href=\"{src}\">{}</a>", escape(text));
            }
        }
        Node::ExternalLink {
            url,
            title,
            children,
        } => {
            let _ = write!(html, "<a href=\"{}\"", escape(url.as_str()));
            if let Some(title) = title {
                let _ = write!(html, " title=\"{}\"", escape(title));
            }
            html.push('>');
            html.push_str(&to_html(children));
            html.push_str("</a>");
        }
        Node::ExternalImage { url, alt, title } => {
            let _ = write!(
                html,
                "<img src=\"{}\" alt=\"{}\"",
                escape(url.as_str()),
                escape(alt.as_deref().unwrap_or(""))
            );
            if let Some(title) = title {
                let _ = write!(html, " title=\"{}\"", escape(title));
            }
            html.push('>');
        }
        // Unresolved references never reach a renderer: the filter replaced each of them.
        _ => {}
    }
}

fn page_href(slug: &str, anchor: Option<&Anchor>) -> String {
    match anchor {
        Some(anchor) => format!("/{slug}#{}", anchor.as_str()),
        None => format!("/{slug}"),
    }
}

/// `text` with `&`, `<`, `>`, `"` and `'` escaped for HTML text and attribute values.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
