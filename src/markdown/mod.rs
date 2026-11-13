//! Markdown logic that doesn't depend on GTK: syntax scanning for the source editor, the
//! formatting tools, Return in lists and quotes, and the formatting in effect at the caret.
//!
//! Offsets in `TextEdit` count characters (Unicode scalar values), the unit `GtkTextBuffer`
//! uses for `iter_at_offset`. The scanner works on line slices and reports byte ranges,
//! which the highlighter turns into iters with `iter_at_line_index`.

pub mod active;
pub mod continuation;
pub mod format;
pub mod model;
pub mod parse;
pub mod serialize;
pub mod syntax;

use std::ops::Range;

/// A change to apply to the text, in character offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub replacement: String,
    /// The selection once the replacement is in place.
    pub selection: Range<usize>,
}

/// A change in byte offsets, as the editing logic works it out on `&str`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ByteEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

impl ByteEdit {
    /// The same edit in character offsets; `selection` is measured in the edited text.
    pub fn to_chars(&self, text: &str) -> TextEdit {
        let start = char_offset(text, self.range.start);
        let end = start + text[self.range.clone()].chars().count();
        let prefix = &text[..self.range.start];
        let edited = format!("{prefix}{}{}", self.replacement, &text[self.range.end..]);
        TextEdit {
            range: start..end,
            replacement: self.replacement.clone(),
            selection: char_offset(&edited, self.selection.start)..char_offset(&edited, self.selection.end),
        }
    }
}

/// The byte index of a character offset, clamped to the end of the text.
pub fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

/// The character offset of a byte index (which must be on a character boundary).
pub fn char_offset(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// A character range as a byte range, clamped to the text.
pub fn byte_range(text: &str, chars: Range<usize>) -> Range<usize> {
    let start = byte_index(text, chars.start);
    let len = chars.end.saturating_sub(chars.start);
    let end = text[start..].char_indices().nth(len).map_or(text.len(), |(i, _)| start + i);
    start..end
}

/// The line around a byte index, line break included, as `NSString.lineRange` gives it.
pub(crate) fn line_range(text: &str, at: usize) -> Range<usize> {
    let at = at.min(text.len());
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i + 1);
    start..end
}

/// A line range without its line break.
pub(crate) fn content_range(text: &str, line: Range<usize>) -> Range<usize> {
    let mut end = line.end;
    while end > line.start && matches!(text.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    line.start..end
}

/// The smallest replacement that turns `old` into `new`: what lies between their common
/// start and their common end, in characters. `None` when they are equal.
pub fn difference(old: &str, new: &str) -> Option<(Range<usize>, String)> {
    if old == new {
        return None;
    }
    let prefix: usize = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let old_rest: Vec<char> = old.chars().skip(prefix).collect();
    let new_rest: Vec<char> = new.chars().skip(prefix).collect();
    let suffix = old_rest.iter().rev().zip(new_rest.iter().rev()).take_while(|(a, b)| a == b).count();
    let removed = old_rest.len() - suffix;
    let inserted: String = new_rest[..new_rest.len() - suffix].iter().collect();
    Some((prefix..prefix + removed, inserted))
}

#[cfg(test)]
pub(crate) mod marks {
    //! Selections written inline in tests: `«` and `»` around selected text, `‸` for the caret.
    use super::TextEdit;
    use std::ops::Range;

    pub fn unmark(marked: &str) -> (String, Range<usize>) {
        let chars: Vec<char> = marked.chars().collect();
        if let Some(caret) = chars.iter().position(|&c| c == '‸') {
            return (marked.replacen('‸', "", 1), caret..caret);
        }
        let start = chars.iter().position(|&c| c == '«').expect("no selection marks");
        let end = chars.iter().position(|&c| c == '»').expect("no selection end");
        (marked.replace(['«', '»'], ""), start..end - 1)
    }

    pub fn mark(text: &str, selection: Range<usize>) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        if selection.is_empty() {
            chars.insert(selection.start, '‸');
        } else {
            chars.insert(selection.end, '»');
            chars.insert(selection.start, '«');
        }
        chars.into_iter().collect()
    }

    pub fn apply(text: &str, edit: &TextEdit) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out: String = chars[..edit.range.start].iter().collect();
        out.push_str(&edit.replacement);
        out.extend(&chars[edit.range.end..]);
        mark(&out, edit.selection.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_characters() {
        let text = "🌮 ñ a";
        assert_eq!(byte_index(text, 2), 5);
        assert_eq!(char_offset(text, 5), 2);
        assert_eq!(byte_range(text, 2..3), 5..7);
        assert_eq!(byte_index(text, 99), text.len());
    }

    #[test]
    fn minimal_difference() {
        assert_eq!(difference("abc", "abc"), None);
        assert_eq!(difference("hola mundo", "hola nuevo mundo"), Some((5..5, "nuevo ".into())));
        assert_eq!(difference("hola nuevo mundo", "hola mundo"), Some((5..11, String::new())));
        // ambiguous cases only need to give the new text back
        let (old, new) = ("a **b** c", "a *b* c");
        let (range, inserted) = difference(old, new).unwrap();
        let chars: Vec<char> = old.chars().collect();
        let applied: String = chars[..range.start].iter().chain(inserted.chars().collect::<Vec<_>>().iter()).chain(chars[range.end..].iter()).collect();
        assert_eq!(applied, new);
        assert_eq!(difference("🌮 a", "🌮 b"), Some((2..3, "b".into())));
        assert_eq!(difference("ñ", "ññ"), Some((1..1, "ñ".into())));
    }

    #[test]
    fn lines() {
        let text = "uno\ndos\n";
        assert_eq!(line_range(text, 5), 4..8);
        assert_eq!(line_range(text, 8), 8..8);
        assert_eq!(content_range(text, 4..8), 4..7);
    }
}
