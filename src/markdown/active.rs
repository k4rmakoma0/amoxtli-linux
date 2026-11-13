//! The formatting in effect at the caret or across the selection, which the toolbar and the
//! menus show as pressed buttons and checkmarks.

use super::syntax::{scan_line, SyntaxKind};
use super::{byte_range, content_range, line_range};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bulleted,
    Numbered,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActiveFormats {
    pub strong: bool,
    pub emphasis: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: bool,
    /// 0 for body text.
    pub heading: u8,
    pub list: Option<ListKind>,
    pub quote: bool,
    pub code_block: bool,
}

/// Reads the active formatting from Markdown source, from the syntax around the caret. Only
/// the caret's line is scanned, so moving the caret stays cheap in long documents;
/// `starts_in_fence` says whether that line begins inside a fenced code block.
pub fn source_formats(text: &str, selection: Range<usize>, starts_in_fence: bool) -> ActiveFormats {
    let selection = byte_range(text, selection);
    let line = content_range(text, line_range(text, selection.start));
    let mut spans = vec![];
    let ends_in_fence = scan_line(&text[line.clone()], starts_in_fence, &mut spans);
    // The opening fence, the code and the closing fence are all part of the block.
    let mut formats = ActiveFormats { code_block: starts_in_fence || ends_in_fence, ..Default::default() };
    let local = selection.start.saturating_sub(line.start)..selection.end.saturating_sub(line.start);
    for span in spans {
        let covered = covers(&span.range, &local);
        match span.kind {
            SyntaxKind::Heading(level) => formats.heading = level,
            SyntaxKind::Quote => formats.quote = true,
            SyntaxKind::ListMarker => {
                let numbered = text[line.start + span.range.start..].starts_with(|c: char| c.is_ascii_digit());
                formats.list = Some(if numbered { ListKind::Numbered } else { ListKind::Bulleted });
            }
            SyntaxKind::Strong if covered => formats.strong = true,
            SyntaxKind::Emphasis if covered => formats.emphasis = true,
            SyntaxKind::Underline if covered => formats.underline = true,
            SyntaxKind::Strikethrough if covered => formats.strikethrough = true,
            SyntaxKind::InlineCode if covered => formats.code = true,
            SyntaxKind::LinkText if covered => formats.link = true,
            _ => {}
        }
    }
    formats
}

/// A caret counts from the start of the styled text to its end, between the markers, which
/// is where typing takes that style. A selection must lie within the styled text.
fn covers(styled: &Range<usize>, selection: &Range<usize>) -> bool {
    if selection.is_empty() {
        return selection.start >= styled.start && selection.start <= styled.end;
    }
    selection.start >= styled.start && selection.end <= styled.end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::marks::unmark;

    fn formats(marked: &str) -> ActiveFormats {
        formats_in(marked, false)
    }

    fn formats_in(marked: &str, starts_in_fence: bool) -> ActiveFormats {
        let (text, selection) = unmark(marked);
        source_formats(&text, selection, starts_in_fence)
    }

    fn inline(f: ActiveFormats) -> [bool; 5] {
        [f.strong, f.emphasis, f.underline, f.strikethrough, f.code]
    }

    #[test]
    fn plain_text_has_nothing_active() {
        assert_eq!(formats("texto‸ normal"), ActiveFormats::default());
    }

    #[test]
    fn caret_inside_and_at_the_edges_of_bold() {
        assert!(formats("a **ne‸grita** b").strong);
        assert!(formats("a **‸negrita** b").strong);
        assert!(formats("a **negrita‸** b").strong);
        assert!(!formats("a ‸**negrita** b").strong);
        assert!(!formats("a **negrita**‸ b").strong);
    }

    #[test]
    fn nested_styles() {
        assert_eq!(inline(formats("**negrita y _cur‸siva_**")), [true, true, false, false, false]);
    }

    #[test]
    fn selection_must_lie_within_the_style() {
        assert!(formats("**«negrita»** b").strong);
        assert!(!formats("**nueva «negrita** b»").strong);
    }

    #[test]
    fn underline_strikethrough_code_and_link() {
        assert!(formats("<u>sub‸rayado</u>").underline);
        assert!(formats("~~ta‸chado~~").strikethrough);
        assert!(formats("`có‸digo`").code);
        assert!(formats("[en‸lace](https://a.b)").link);
    }

    #[test]
    fn heading_level() {
        assert_eq!(formats("### Tí‸tulo").heading, 3);
        assert_eq!(formats("Tí‸tulo").heading, 0);
    }

    #[test]
    fn lists_and_quote() {
        assert_eq!(formats("- uno‸").list, Some(ListKind::Bulleted));
        assert_eq!(formats("12. uno‸").list, Some(ListKind::Numbered));
        assert!(formats("> ci‸ta").quote);
        assert_eq!(formats("> - uno‸").list, Some(ListKind::Bulleted));
    }

    #[test]
    fn only_the_carets_line_counts() {
        assert_eq!(formats("# Título\n\npárrafo‸").heading, 0);
    }

    #[test]
    fn code_block_from_its_fences_and_content() {
        assert!(formats("```swi‸ft\nlet a").code_block);
        assert!(formats_in("```\nlet ‸a\n```", true).code_block);
        assert_eq!(inline(formats("`**no**` y ‸ya")), [false; 5]);
    }
}
