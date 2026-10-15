//! The text of a note before it is parsed: the frontmatter is cut off, and every comment is removed.
//!
//! Obsidian comments (`%% … %%`) and HTML comments (`<!-- … -->`) are removed, inline or over
//! several lines. A comment marker inside a fenced code block or an inline code span is kept as code
//! (workspace spec `site-release-privacy`, requirement "Comments are removed before rendering"). An
//! unclosed comment runs to the end of the note, as in Obsidian.

/// A comment that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comment {
    Obsidian,
    Html,
}

impl Comment {
    const fn close(self) -> &'static str {
        match self {
            Self::Obsidian => "%%",
            Self::Html => "-->",
        }
    }
}

/// A fenced code block that is open: its fence character and the length of its fence.
#[derive(Debug, Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

/// The body of `text`: the text after a frontmatter block that starts on the first line.
pub(crate) fn body_of(text: &str) -> &str {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return text;
    }
    let mut consumed = text.find('\n').map_or(text.len(), |at| at + 1);
    for line in lines {
        consumed += line.len();
        let trimmed = line.trim_end();
        if trimmed == "---" || trimmed == "..." {
            return text.get(consumed..).unwrap_or("");
        }
    }
    // An unclosed frontmatter block is no frontmatter.
    text
}

/// `body` with every comment removed. The line breaks of a comment over several lines stay, so the
/// blocks around it stay separate.
pub(crate) fn without_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<Fence> = None;
    let mut comment: Option<Comment> = None;
    for line in body.split_inclusive('\n') {
        if comment.is_none() {
            if let Some(open) = fence {
                out.push_str(line);
                if closes(line, open) {
                    fence = None;
                }
                continue;
            }
            if let Some(open) = opens(line) {
                fence = Some(open);
                out.push_str(line);
                continue;
            }
        }
        strip_line(line, &mut out, &mut comment);
    }
    out
}

/// The fence that `line` opens: up to three spaces, then three or more backticks or tildes.
fn opens(line: &str) -> Option<Fence> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = line.trim_start_matches(' ');
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = rest.chars().take_while(|c| *c == marker).count();
    (length >= 3).then_some(Fence { marker, length })
}

/// Whether `line` closes `fence`: the same character, at least as many, and nothing after it.
fn closes(line: &str, fence: Fence) -> bool {
    let rest = line.trim_start_matches(' ');
    let length = rest.chars().take_while(|c| *c == fence.marker).count();
    length >= fence.length && rest.chars().skip(length).all(char::is_whitespace)
}

/// Copy `line` to `out` without its comments, outside inline code spans.
fn strip_line(line: &str, out: &mut String, comment: &mut Option<Comment>) {
    let mut rest = line;
    loop {
        if let Some(open) = *comment {
            if let Some(at) = rest.find(open.close()) {
                rest = rest.get(at + open.close().len()..).unwrap_or("");
                *comment = None;
                continue;
            }
            if rest.ends_with('\n') {
                out.push('\n');
            }
            return;
        }
        let Some(at) = next_marker(rest) else {
            out.push_str(rest);
            return;
        };
        out.push_str(rest.get(..at).unwrap_or(""));
        rest = rest.get(at..).unwrap_or("");
        if let Some(after) = rest.strip_prefix("%%") {
            *comment = Some(Comment::Obsidian);
            rest = after;
        } else if let Some(after) = rest.strip_prefix("<!--") {
            *comment = Some(Comment::Html);
            rest = after;
        } else {
            rest = copy_code_span(rest, out);
        }
    }
}

/// The byte offset of the next backtick, `%%` or `<!--` in `text`.
fn next_marker(text: &str) -> Option<usize> {
    text.char_indices().map(|(at, _)| at).find(|at| {
        text.get(*at..).is_some_and(|rest| {
            rest.starts_with('`') || rest.starts_with("%%") || rest.starts_with("<!--")
        })
    })
}

/// Copy the inline code span at the start of `text` (a run of backticks, the code, and a run of
/// as many backticks) to `out`, and return what follows it. A run without a closing run is text.
fn copy_code_span<'a>(text: &'a str, out: &mut String) -> &'a str {
    let run = text.chars().take_while(|c| *c == '`').count();
    let after = text.get(run..).unwrap_or("");
    let mut search = 0;
    while let Some(found) = after.get(search..).and_then(|rest| rest.find('`')) {
        let start = search + found;
        let length = after
            .get(start..)
            .map_or(0, |rest| rest.chars().take_while(|c| *c == '`').count());
        if length == run {
            let end = run + start + length;
            out.push_str(text.get(..end).unwrap_or(""));
            return text.get(end..).unwrap_or("");
        }
        search = start + length;
    }
    out.push_str(text.get(..run).unwrap_or(""));
    after
}
