//! What Return does on a list or quote line of Markdown source: the next line starts with
//! the same markers, the next number, the same indentation. On an item with no text, Return
//! removes its marker instead, which is how a list ends.

use super::{byte_range, content_range, line_range, ByteEdit, TextEdit};
use fancy_regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([ \t]*)((?:>[ \t]?)*)([ \t]*)(?:([-*+])|(\d{1,9})([.)]))?([ \t]+)?").expect("prefix pattern")
});

/// `None` when Return should only insert a line break: outside lists and quotes, inside a
/// fenced code block, with the caret before the line's text, or with a selection that
/// reaches into another line.
pub fn edit(text: &str, selection: Range<usize>, starts_in_fence: bool) -> Option<TextEdit> {
    if starts_in_fence {
        return None;
    }
    let selection = byte_range(text, selection);
    let line = content_range(text, line_range(text, selection.start));
    if selection.end > line.end {
        return None;
    }
    let content = &text[line.clone()];
    let prefix = LinePrefix::parse(content)?;
    if !prefix.is_continued() || selection.start < line.start + prefix.length {
        return None;
    }
    let rest = &content[prefix.length..];
    let edit = if rest.trim().is_empty() && selection.is_empty() {
        let remaining = prefix.without_innermost_marker();
        let caret = line.start + remaining.len();
        ByteEdit { range: line, replacement: remaining, selection: caret..caret }
    } else {
        let next = format!("\n{}", prefix.continuation());
        let caret = selection.start + next.len();
        ByteEdit { range: selection, replacement: next, selection: caret..caret }
    };
    Some(edit.to_chars(text))
}

enum Marker {
    Bullet(char),
    Number(u64, char),
}

/// The indentation, quote markers and list marker at the start of a line.
struct LinePrefix {
    indentation: String,
    quotes: String,
    inner_indentation: String,
    marker: Option<Marker>,
    /// In bytes.
    length: usize,
}

impl LinePrefix {
    fn parse(line: &str) -> Option<Self> {
        let caps = PREFIX.captures(line).ok().flatten()?;
        let group = |i| caps.get(i).map_or("", |m| m.as_str());
        // A marker only counts with a space after it: `-text` and `1.5` are text.
        let spaced = caps.get(7).is_some();
        let marker = if !spaced {
            None
        } else if let Some(bullet) = group(4).chars().next() {
            Some(Marker::Bullet(bullet))
        } else if let (Ok(value), Some(delimiter)) = (group(5).parse(), group(6).chars().next()) {
            Some(Marker::Number(value, delimiter))
        } else {
            None
        };
        let (indentation, quotes) = (group(1).to_owned(), group(2).to_owned());
        let length = if marker.is_none() { indentation.len() + quotes.len() } else { caps.get(0).unwrap().end() };
        Some(Self { indentation, quotes, inner_indentation: group(3).to_owned(), marker, length })
    }

    fn is_continued(&self) -> bool {
        self.marker.is_some() || !self.quotes.is_empty()
    }

    /// The start of the next line: the same nesting, and the next marker.
    fn continuation(&self) -> String {
        let quotes = if self.quotes.is_empty() || self.quotes.ends_with(' ') {
            self.quotes.clone()
        } else {
            format!("{} ", self.quotes)
        };
        let marker = match self.marker {
            Some(Marker::Bullet(c)) => format!("{c} "),
            Some(Marker::Number(n, d)) => format!("{}{d} ", n + 1),
            None => String::new(),
        };
        format!("{}{quotes}{}{marker}", self.indentation, self.inner_indentation)
    }

    /// An empty item loses its list marker; an empty quote line loses its quote.
    fn without_innermost_marker(&self) -> String {
        match self.marker {
            None => String::new(),
            Some(_) => format!("{}{}", self.indentation, self.quotes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::marks::{apply, unmark};

    fn press_return(marked: &str) -> Option<String> {
        press_return_in(marked, false)
    }

    fn press_return_in(marked: &str, starts_in_fence: bool) -> Option<String> {
        let (text, selection) = unmark(marked);
        edit(&text, selection, starts_in_fence).map(|e| apply(&text, &e))
    }

    #[test]
    fn bullet_continues() {
        assert_eq!(press_return("- uno‸").as_deref(), Some("- uno\n- ‸"));
        assert_eq!(press_return("* uno‸").as_deref(), Some("* uno\n* ‸"));
    }

    #[test]
    fn number_goes_up() {
        assert_eq!(press_return("1. uno‸").as_deref(), Some("1. uno\n2. ‸"));
        assert_eq!(press_return("9) nueve‸").as_deref(), Some("9) nueve\n10) ‸"));
    }

    #[test]
    fn indentation_and_quotes_are_kept() {
        assert_eq!(press_return("  - anidado‸").as_deref(), Some("  - anidado\n  - ‸"));
        assert_eq!(press_return("> - en cita‸").as_deref(), Some("> - en cita\n> - ‸"));
        assert_eq!(press_return("> cita‸").as_deref(), Some("> cita\n> ‸"));
    }

    #[test]
    fn text_after_the_caret_moves_to_the_new_item() {
        assert_eq!(press_return("- uno‸ dos").as_deref(), Some("- uno\n- ‸ dos"));
    }

    #[test]
    fn empty_item_ends_the_list() {
        assert_eq!(press_return("- uno\n- ‸").as_deref(), Some("- uno\n‸"));
        assert_eq!(press_return("> - ‸").as_deref(), Some("> ‸"));
        assert_eq!(press_return("> ‸").as_deref(), Some("‸"));
    }

    #[test]
    fn plain_text_and_headings_are_left_to_the_text_view() {
        assert_eq!(press_return("texto‸"), None);
        assert_eq!(press_return("# Título‸"), None);
        assert_eq!(press_return("-sin espacio‸"), None);
        assert_eq!(press_return("1.5 no es lista‸"), None);
    }

    #[test]
    fn caret_before_the_text_leaves_a_line_break() {
        assert_eq!(press_return("‸- uno"), None);
    }

    #[test]
    fn code_blocks_are_left_alone() {
        assert_eq!(press_return_in("- dentro de código‸", true), None);
    }

    #[test]
    fn selection_across_lines_is_left_alone() {
        assert_eq!(press_return("- u«no\n- d»os"), None);
    }

    #[test]
    fn selection_on_one_line_is_replaced() {
        assert_eq!(press_return("- u«no» dos").as_deref(), Some("- u\n- ‸ dos"));
    }

    #[test]
    fn multibyte_text() {
        assert_eq!(press_return("- ñandú 🌮‸").as_deref(), Some("- ñandú 🌮\n- ‸"));
    }
}
