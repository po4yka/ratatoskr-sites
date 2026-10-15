//! The closed element and attribute allowlist of the HTML body of a page (workspace spec
//! `site-release-privacy`, requirement "Rendered HTML is sanitised").
//!
//! The whole body passes the allowlist, so raw HTML of a note gets the same treatment as the markup
//! of the renderer. The tokenizer of the HTML standard (`html5ever`) reads the body as a browser
//! does, and only these tokens are written again:
//!
//! - a start or end tag of an element in [`ELEMENTS`], with the attributes of [`ATTRIBUTES`] only;
//!   an end tag only when its element is open;
//! - text, escaped again.
//!
//! An element of [`REMOVED_WITH_CONTENT`] is removed with everything in it; any other element
//! outside the allowlist, `embed` among them, loses its tags and keeps its text. Comments and doctypes are removed. A URL
//! attribute keeps only an `https`, `http` or `mailto` URL or a relative reference; a browser
//! removes tabs and line breaks from a URL, so the check does too. An `input` stays only as a task
//! list box.

use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// The elements that a page body may hold.
const ELEMENTS: [&str; 44] = [
    "a",
    "abbr",
    "b",
    "blockquote",
    "br",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "input",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "s",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];

/// The elements without an end tag.
const VOID: [&str; 4] = ["br", "hr", "img", "input"];

/// The elements that are removed with their content, not only with their tags.
const REMOVED_WITH_CONTENT: [&str; 15] = [
    "script", "style", "iframe", "object", "template", "noscript", "noembed", "noframes",
    "textarea", "title", "xmp", "select", "svg", "math", "form",
];

/// The attributes that an element may carry. No element carries any other attribute.
const ATTRIBUTES: [(&str, &[&str]); 14] = [
    ("a", &["href", "title", "data-wikilink", "data-embed"]),
    ("img", &["src", "alt", "title"]),
    ("div", &["class", "data-callout", "data-callout-fold"]),
    ("code", &["class"]),
    ("ol", &["start"]),
    ("td", &["align"]),
    ("th", &["align"]),
    ("input", &["type", "checked", "disabled"]),
    ("h1", &["id"]),
    ("h2", &["id"]),
    ("h3", &["id"]),
    ("h4", &["id"]),
    ("h5", &["id"]),
    ("h6", &["id"]),
];

/// The most elements that may be open at once. A deeper start tag is dropped and its text stays,
/// so the work of an end tag, which looks for its element among the open ones, has a bound.
pub const MAX_DEPTH: usize = 256;

/// The attributes that are written without a value, as the renderer writes them.
const BOOLEAN: [&str; 4] = ["checked", "disabled", "data-wikilink", "data-embed"];

/// The attributes whose value is a URL.
const URL_ATTRIBUTES: [&str; 2] = ["href", "src"];

/// The URL schemes that may stay.
const SCHEMES: [&str; 3] = ["https", "http", "mailto"];

/// The body of `html` after the allowlist.
#[must_use]
pub fn sanitise(html: &str) -> String {
    let tokenizer = Tokenizer::new(Writer::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let mut state = tokenizer.sink.state.borrow_mut();
    while let Some(open) = state.open.pop() {
        state.out.push_str("</");
        state.out.push_str(&open);
        state.out.push('>');
    }
    std::mem::take(&mut state.out)
}

#[derive(Debug, Default)]
struct Writer {
    state: RefCell<State>,
}

#[derive(Debug, Default)]
struct State {
    out: String,
    /// The allowlisted elements that are open, innermost last.
    open: Vec<String>,
    /// The removed element whose content is skipped, and how deep it is nested in itself.
    skipping: Option<(String, usize)>,
}

impl TokenSink for Writer {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        let mut state = self.state.borrow_mut();
        match token {
            Token::TagToken(tag) => return state.tag(&tag),
            Token::CharacterTokens(text) if state.skipping.is_none() => {
                state.out.push_str(&escape(&text));
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

impl State {
    fn tag(&mut self, tag: &Tag) -> TokenSinkResult<()> {
        let name: &str = &tag.name;
        if let Some((skipped, depth)) = &mut self.skipping {
            if skipped == name {
                match tag.kind {
                    TagKind::StartTag => *depth += 1,
                    TagKind::EndTag if *depth > 1 => *depth -= 1,
                    TagKind::EndTag => self.skipping = None,
                }
            }
            return TokenSinkResult::Continue;
        }
        if REMOVED_WITH_CONTENT.contains(&name) {
            // A browser ignores the self-closing flag of these elements, so `<script/>` starts
            // content too.
            if tag.kind == TagKind::StartTag {
                self.skipping = Some((name.to_owned(), 1));
                return raw_kind(name);
            }
            return TokenSinkResult::Continue;
        }
        if !ELEMENTS.contains(&name) {
            return TokenSinkResult::Continue;
        }
        match tag.kind {
            TagKind::StartTag => self.start(name, tag),
            TagKind::EndTag => self.end(name),
        }
        TokenSinkResult::Continue
    }

    fn start(&mut self, name: &str, tag: &Tag) {
        if !VOID.contains(&name) && self.open.len() >= MAX_DEPTH {
            return;
        }
        if name == "input" && attribute(tag, "type").as_deref() != Some("checkbox") {
            return;
        }
        self.out.push('<');
        self.out.push_str(name);
        let allowed = ATTRIBUTES
            .iter()
            .find(|(element, _)| *element == name)
            .map_or(&[][..], |(_, attributes)| *attributes);
        for attribute in &tag.attrs {
            let key: &str = &attribute.name.local;
            if !attribute.name.ns.is_empty() || !allowed.contains(&key) {
                continue;
            }
            let Some(value) = allowed_value(name, key, &attribute.value) else {
                continue;
            };
            self.out.push(' ');
            self.out.push_str(key);
            if BOOLEAN.contains(&key) {
                continue;
            }
            self.out.push_str("=\"");
            self.out.push_str(&escape(&value));
            self.out.push('"');
        }
        self.out.push('>');
        // A browser ignores the self-closing flag of a non-void element: `<div/>` opens a div. So
        // every non-void start tag opens, and the writer closes it.
        if !VOID.contains(&name) {
            self.open.push(name.to_owned());
        }
    }

    fn end(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|open| open == name) else {
            return;
        };
        for open in self.open.split_off(at).into_iter().rev() {
            self.out.push_str("</");
            self.out.push_str(&open);
            self.out.push('>');
        }
    }
}

/// The state that the tokenizer takes for the content of a removed element, as a browser does.
fn raw_kind(name: &str) -> TokenSinkResult<()> {
    match name {
        "script" => TokenSinkResult::RawData(RawKind::ScriptData),
        "textarea" | "title" => TokenSinkResult::RawData(RawKind::Rcdata),
        "style" | "iframe" | "noscript" | "noembed" | "noframes" | "xmp" => {
            TokenSinkResult::RawData(RawKind::Rawtext)
        }
        _ => TokenSinkResult::Continue,
    }
}

fn attribute(tag: &Tag, key: &str) -> Option<String> {
    tag.attrs
        .iter()
        .find(|attribute| &*attribute.name.local == key)
        .map(|attribute| attribute.value.to_ascii_lowercase())
}

/// The classes that the renderer writes on a `div`.
const DIV_CLASSES: [&str; 3] = ["callout", "callout-title", "callout-content"];

/// The longest language of a code block in its class `language-<lang>`.
const MAX_LANGUAGE: usize = 32;

/// The value of the attribute `key` of `element` that may stay, or `None` when the attribute is
/// removed.
fn allowed_value(element: &str, key: &str, value: &str) -> Option<String> {
    match key {
        "class" => return allowed_class(element, value),
        "id" => return crate::ids::is_heading_id(value).then(|| value.to_owned()),
        "src" => return allowed_image(value),
        _ => {}
    }
    if !URL_ATTRIBUTES.contains(&key) {
        return Some(value.to_owned());
    }
    let url: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let scheme_end = url.find(':');
    let path_start = url.find(['/', '?', '#']);
    match (scheme_end, path_start) {
        (Some(colon), path) if path.is_none_or(|path| colon < path) => {
            let scheme = url.get(..colon)?.to_ascii_lowercase();
            SCHEMES.contains(&scheme.as_str()).then_some(url)
        }
        _ => Some(url),
    }
}

/// The classes of `value` that the renderer writes on `element`: the callout classes on a `div`,
/// and one `language-<lang>` on a `code`, with a lang of at most [`MAX_LANGUAGE`] lower-case ASCII
/// letters, digits and `+`, `#`, `.`, `_`, `-`.
fn allowed_class(element: &str, value: &str) -> Option<String> {
    let kept: Vec<&str> = match element {
        "div" => value
            .split_ascii_whitespace()
            .filter(|class| DIV_CLASSES.contains(class))
            .collect(),
        "code" => value
            .split_ascii_whitespace()
            .filter(|class| {
                class.strip_prefix("language-").is_some_and(|lang| {
                    (1..=MAX_LANGUAGE).contains(&lang.len())
                        && lang.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'+' | b'#' | b'.' | b'_' | b'-')
                        })
                })
            })
            .take(1)
            .collect(),
        _ => Vec::new(),
    };
    (!kept.is_empty()).then(|| kept.join(" "))
}

/// The source of an image that may stay: an asset of the release, `assets/<sha256>`, or an
/// `https:` URL. No `http:`, no protocol-relative URL and no other path of the site's origin.
fn allowed_image(value: &str) -> Option<String> {
    let url: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let asset = url.strip_prefix("assets/").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    let https = url
        .get(..6)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https:"));
    (asset || https).then_some(url)
}

/// `text` with `&`, `<`, `>`, `"` and `'` escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\0' => {}
            other => out.push(other),
        }
    }
    out
}
