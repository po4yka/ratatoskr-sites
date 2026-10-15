//! The id of a heading, one rule for the renderer and the HTML allowlist: lower-case ASCII
//! letters, digits and `-`, at most 255 characters. The contract fixtures of `sites/page.v1` write
//! the id without a prefix (`id="start-here"`), so none is added.

/// The longest heading id.
const MAX_LENGTH: usize = 255;

/// The id of a heading with the text `text`: ASCII letters in lower case and digits, and one `-`
/// for each run of other characters, without a `-` at either end. `None` when nothing is left.
#[must_use]
pub fn heading_id(text: &str) -> Option<String> {
    let mut id = String::with_capacity(text.len());
    let mut gap = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if gap && !id.is_empty() {
                id.push('-');
            }
            id.push(character.to_ascii_lowercase());
            gap = false;
        } else {
            gap = true;
        }
        if id.len() >= MAX_LENGTH {
            break;
        }
    }
    id.truncate(MAX_LENGTH);
    let id = id.trim_end_matches('-').to_owned();
    (!id.is_empty()).then_some(id)
}

/// Whether `value` is a heading id that the allowlist keeps.
#[must_use]
pub fn is_heading_id(value: &str) -> bool {
    (1..=MAX_LENGTH).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
