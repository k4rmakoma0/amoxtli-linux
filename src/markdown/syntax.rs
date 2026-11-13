//! Finds the syntax to highlight in the source editor. It works line by line, so that an
//! edit only needs its own lines scanned again; the one state carried between lines is
//! whether they are inside a fenced code block.

use fancy_regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    Heading(u8),
    /// Characters that are Markdown syntax rather than text, such as `**` or `#`.
    Marker,
    Strong,
    Emphasis,
    Underline,
    Strikethrough,
    InlineCode,
    CodeBlock,
    LinkText,
    LinkDestination,
    Quote,
    ListMarker,
    HorizontalRule,
}

/// A styled range, in bytes from the start of the scanned line (or text, for `scan`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SyntaxKind,
}

#[cfg_attr(not(test), allow(dead_code))] // whole-text scans: tests, and code blocks next
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub spans: Vec<Span>,
    /// Whether the text leaves a fenced code block open.
    pub ends_in_fence: bool,
    /// Every line, line break included, after which a fenced code block is open.
    pub fenced_lines: Vec<Range<usize>>,
}

/// The patterns are fixed, so a failure here is a mistake in this file that tests catch.
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("invalid syntax pattern {pattern}: {e}"))
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}(?:```|~~~)"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}(#{1,6})(?:[ \t]+|$)"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}(?:> ?)+"));
static LIST: LazyLock<Regex> = LazyLock::new(|| re(r"^[ \t]*([-*+]|\d{1,9}[.)])[ \t]+"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"(`+)(?!`)(.+?)(?<!`)(\1)(?!`)"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"(!?\[)([^\]\n]*)(\]\()([^)\n]*)(\))"));
static WRAPPED: LazyLock<Vec<(Regex, SyntaxKind)>> = LazyLock::new(|| {
    vec![
        (re(r"(\*\*|__)(?=\S)(.+?)(?<=\S)(\1)"), SyntaxKind::Strong),
        // One pattern per marker: each only has to stay clear of its own character, so
        // `_italic_` can sit right inside `**bold**` and `*a*` right next to `**`.
        (re(r"(?<![*\\])(\*)(?![*\s])(.+?)(?<![*\s\\])(\*)(?!\*)"), SyntaxKind::Emphasis),
        (re(r"(?<![_\w\\])(_)(?![_\s])(.+?)(?<![_\s\\])(_)(?![_\w])"), SyntaxKind::Emphasis),
        (re(r"(~~)(?=\S)(.+?)(?<=\S)(~~)"), SyntaxKind::Strikethrough),
        (re(r"(?i)(<u>)(.+?)(</u>)"), SyntaxKind::Underline),
    ]
});

/// Whether a line (without its line break) opens or closes a fenced code block.
pub fn is_fence(line: &str) -> bool {
    FENCE.is_match(line).unwrap_or(false)
}

/// What follows the backticks or tildes of a fence line, as `swift` in ```` ```swift ````.
pub fn fence_info(line: &str) -> &str {
    line.trim_start_matches(' ').trim_start_matches(['`', '~']).trim()
}

/// Scans one line, without its line break. Spans are added in bytes from the line start.
/// Returns whether the next line is inside a fenced code block.
pub fn scan_line(line: &str, in_fence: bool, out: &mut Vec<Span>) -> bool {
    if is_fence(line) {
        push(out, 0..line.len(), SyntaxKind::Marker);
        return !in_fence;
    }
    if in_fence {
        push(out, 0..line.len(), SyntaxKind::CodeBlock);
        return true;
    }
    scan_block(line, out);
    false
}

fn push(out: &mut Vec<Span>, range: Range<usize>, kind: SyntaxKind) {
    if !range.is_empty() {
        out.push(Span { range, kind });
    }
}

fn scan_block(line: &str, out: &mut Vec<Span>) {
    if let Ok(Some(caps)) = HEADING.captures(line) {
        let whole = caps.get(0).unwrap();
        let level = caps.get(1).unwrap().as_str().len() as u8;
        push(out, 0..line.len(), SyntaxKind::Heading(level));
        push(out, whole.range(), SyntaxKind::Marker);
        scan_inline(line, whole.end(), out);
        return;
    }
    if RULE.is_match(line).unwrap_or(false) {
        push(out, 0..line.len(), SyntaxKind::HorizontalRule);
        return;
    }
    let mut rest = 0;
    if let Ok(Some(quote)) = QUOTE.find(line) {
        push(out, 0..line.len(), SyntaxKind::Quote);
        push(out, quote.range(), SyntaxKind::Marker);
        rest = quote.end();
    }
    if let Ok(Some(caps)) = LIST.captures(&line[rest..]) {
        let marker = caps.get(1).unwrap();
        push(out, rest + marker.start()..rest + marker.end(), SyntaxKind::ListMarker);
        rest += caps.get(0).unwrap().end();
    }
    scan_inline(line, rest, out);
}

fn scan_inline(line: &str, from: usize, out: &mut Vec<Span>) {
    let text = &line[from..];
    if text.is_empty() {
        return;
    }
    let at = |r: Range<usize>| from + r.start..from + r.end;
    // Code spans go first: nothing inside them is syntax.
    let mut code: Vec<Range<usize>> = vec![];
    for caps in CODE.captures_iter(text).flatten() {
        let whole = caps.get(0).unwrap().range();
        push(out, at(whole.clone()), SyntaxKind::InlineCode);
        push(out, at(caps.get(1).unwrap().range()), SyntaxKind::Marker);
        push(out, at(caps.get(3).unwrap().range()), SyntaxKind::Marker);
        code.push(whole);
    }
    let outside_code = |r: &Range<usize>| !code.iter().any(|c| c.start < r.end && r.start < c.end);
    for caps in LINK.captures_iter(text).flatten() {
        if !outside_code(&caps.get(0).unwrap().range()) {
            continue;
        }
        let kinds = [
            SyntaxKind::Marker,
            SyntaxKind::LinkText,
            SyntaxKind::Marker,
            SyntaxKind::LinkDestination,
            SyntaxKind::Marker,
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            push(out, at(caps.get(i + 1).unwrap().range()), kind);
        }
    }
    for (pattern, kind) in WRAPPED.iter() {
        for caps in pattern.captures_iter(text).flatten() {
            if !outside_code(&caps.get(0).unwrap().range()) {
                continue;
            }
            push(out, at(caps.get(1).unwrap().range()), SyntaxKind::Marker);
            push(out, at(caps.get(2).unwrap().range()), *kind);
            push(out, at(caps.get(3).unwrap().range()), SyntaxKind::Marker);
        }
    }
}

/// Scans every line of a text, with spans in bytes from its start.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scan(text: &str) -> Scan {
    scan_from(text, false)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn scan_from(text: &str, starts_in_fence: bool) -> Scan {
    let mut spans = vec![];
    let mut fenced_lines = vec![];
    let mut in_fence = starts_in_fence;
    let mut start = 0;
    let mut line_spans = vec![];
    loop {
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i + 1);
        let content = super::content_range(text, start..end);
        line_spans.clear();
        in_fence = scan_line(&text[content], in_fence, &mut line_spans);
        spans.extend(line_spans.drain(..).map(|s| Span { range: start + s.range.start..start + s.range.end, kind: s.kind }));
        if in_fence {
            fenced_lines.push(start..end);
        }
        start = end;
        if start >= text.len() {
            break;
        }
    }
    Scan { spans, ends_in_fence: in_fence, fenced_lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SyntaxKind::*;

    fn spans(text: &str, kind: SyntaxKind) -> Vec<Range<usize>> {
        let mut found: Vec<_> = scan(text).spans.into_iter().filter(|s| s.kind == kind).map(|s| s.range).collect();
        found.sort_by_key(|r| r.start);
        found
    }

    #[test]
    fn heading_line_and_marker() {
        assert_eq!(spans("## Hola", Heading(2)), [0..7]);
        assert_eq!(spans("## Hola", Marker), [0..3]);
    }

    #[test]
    fn strong_with_markers() {
        assert_eq!(spans("a **b** c", Strong), [4..5]);
        assert_eq!(spans("a **b** c", Marker), [2..4, 5..7]);
    }

    #[test]
    fn emphasis() {
        assert_eq!(spans("a _b_ y *c*", Emphasis), [3..4, 9..10]);
    }

    #[test]
    fn emphasis_next_to_strong() {
        // Each marker only stays clear of its own character.
        assert_eq!(spans("**_a_**", Strong), [2..5]);
        assert_eq!(spans("**_a_**", Emphasis), [3..4]);
        assert_eq!(spans("_a_**b**", Emphasis), [1..2]);
        assert_eq!(spans("__*a*__", Emphasis), [3..4]);
    }

    #[test]
    fn nothing_inside_code_span_is_syntax() {
        assert_eq!(spans("`*no*`", InlineCode), [0..6]);
        assert_eq!(spans("`*no*`", Emphasis), Vec::<Range<usize>>::new());
    }

    #[test]
    fn fenced_code_is_not_formatted() {
        let text = "```\n*x*\n```\ntexto *y*";
        let result = scan(text);
        assert_eq!(spans(text, CodeBlock), [4..7]);
        assert_eq!(spans(text, Emphasis), [19..20]);
        assert!(!result.ends_in_fence);
        assert_eq!(result.fenced_lines, [0..4, 4..8]);
    }

    #[test]
    fn open_fence_carries_over() {
        assert!(scan("```swift\nlet a").ends_in_fence);
        assert_eq!(fence_info("```swift"), "swift");
        assert_eq!(fence_info("  ~~~ py title=\"a\""), "py title=\"a\"");
    }

    #[test]
    fn lines_inside_fence_when_starting_in_one() {
        let result = scan_from("*x*", true);
        assert_eq!(result.spans, [Span { range: 0..3, kind: CodeBlock }]);
    }

    #[test]
    fn link_text_and_destination() {
        assert_eq!(spans("[a](b)", LinkText), [1..2]);
        assert_eq!(spans("[a](b)", LinkDestination), [4..5]);
    }

    #[test]
    fn list_markers() {
        assert_eq!(spans("- uno", ListMarker), [0..1]);
        assert_eq!(spans("12. uno", ListMarker), [0..3]);
        assert_eq!(spans("> - uno", ListMarker), [2..3]);
        assert_eq!(spans("-sin espacio", ListMarker), Vec::<Range<usize>>::new());
    }

    #[test]
    fn quote() {
        assert_eq!(spans("> cita", Quote), [0..6]);
        assert_eq!(spans("> cita", Marker), [0..2]);
    }

    #[test]
    fn underline_and_strikethrough() {
        assert_eq!(spans("<u>a</u>", Underline), [3..4]);
        assert_eq!(spans("~~a~~", Strikethrough), [2..3]);
    }

    #[test]
    fn horizontal_rule() {
        assert_eq!(spans("texto\n\n---", HorizontalRule), [7..10]);
    }

    #[test]
    fn multibyte_text_keeps_byte_ranges() {
        assert_eq!(spans("ñ **ñ**", Strong), [5..7]);
    }
}
