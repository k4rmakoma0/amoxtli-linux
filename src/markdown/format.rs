//! Applies a formatting tool to Markdown source by writing the syntax for the person. Tools
//! that are already applied are removed instead, so every one of them works as a toggle.

use super::{byte_range, content_range, line_range, ByteEdit, TextEdit};
use fancy_regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

/// A formatting tool's effect, from the toolbar, the Format menu or the context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatCommand {
    /// Level 0 turns the line back into body text.
    Heading(u8),
    Bold,
    Italic,
    Underline,
    Strikethrough,
    InlineCode,
    Quote,
    BulletList,
    NumberedList,
    CodeBlock,
    Table(TableDimensions),
    HorizontalRule,
    Link { text: String, destination: String },
}

/// Total table dimensions. The first row is the Markdown header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDimensions {
    pub columns: usize,
    pub rows: usize,
    /// The header cells' placeholder text, one per column.
    pub headers: Vec<String>,
}

impl TableDimensions {
    pub const COLUMNS: std::ops::RangeInclusive<usize> = 1..=20;
    pub const ROWS: std::ops::RangeInclusive<usize> = 1..=100;

    /// `header(n)` names column `n`, counting from 1. `None` outside the allowed sizes.
    pub fn new(columns: usize, rows: usize, header: impl Fn(usize) -> String) -> Option<Self> {
        (Self::COLUMNS.contains(&columns) && Self::ROWS.contains(&rows))
            .then(|| Self { columns, rows, headers: (1..=columns).map(header).collect() })
    }
}

/// The edit a tool makes to `text` with the selection (in characters) where it is.
pub fn edit(command: &FormatCommand, text: &str, selection: Range<usize>) -> TextEdit {
    let selection = byte_range(text, selection);
    let lines = LineEditor { text, selection: selection.clone() };
    let edit = match command {
        FormatCommand::Bold => inline("**", "**", text, selection),
        FormatCommand::Italic => inline("_", "_", text, selection),
        FormatCommand::Underline => inline("<u>", "</u>", text, selection),
        FormatCommand::Strikethrough => inline("~~", "~~", text, selection),
        FormatCommand::InlineCode => inline("`", "`", text, selection),
        FormatCommand::Heading(level) => lines.heading(*level),
        FormatCommand::Quote => lines.quote(),
        FormatCommand::BulletList => lines.bullet_list(),
        FormatCommand::NumberedList => lines.numbered_list(),
        FormatCommand::CodeBlock => lines.code_block(),
        FormatCommand::Table(dimensions) => insert_block(text, selection, &table(dimensions), dimensions.headers.first().map(String::as_str)),
        FormatCommand::HorizontalRule => insert_block(text, selection, "---", None),
        FormatCommand::Link { text: label, destination } => link(label, destination, selection),
    };
    edit.to_chars(text)
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("invalid pattern {pattern}: {e}"))
}

static HEADING_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}#{1,6}(?:[ \t]+|$)"));
static HEADING_LEVEL: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}(#{1,6})(?:[ \t]|$)"));
static QUOTE_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}> ?"));
static BULLET_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*[-*+][ \t]+"));
static NUMBER_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\d{1,9}[.)][ \t]+"));
static LIST_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^(\s*)(?:[-*+]|\d{1,9}[.)])[ \t]+"));
static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"^ {0,3}(?:```|~~~)"));
static EMPTY: LazyLock<Regex> = LazyLock::new(|| re(r"^"));

fn matches(pattern: &Regex, line: &str) -> bool {
    pattern.is_match(line).unwrap_or(false)
}

fn prefix_len(pattern: &Regex, line: &str) -> usize {
    pattern.find(line).ok().flatten().map_or(0, |m| m.end())
}

fn is_content(line: &str) -> bool {
    line.chars().any(|c| !c.is_whitespace())
}

// ------------------------------------------------------------------ inline styles

/// Wraps the selection in a pair of markers, or removes them when they are already there.
fn inline(open: &str, close: &str, text: &str, selection: Range<usize>) -> ByteEdit {
    let target = trimming_whitespace(text, selection);
    let inner = &text[target.clone()];
    if let Some(outer) = markers_around(open, close, text, &target) {
        return ByteEdit { selection: outer.start..outer.start + inner.len(), range: outer, replacement: inner.to_owned() };
    }
    if inner.len() >= open.len() + close.len() && inner.starts_with(open) && inner.ends_with(close) {
        let unwrapped = &inner[open.len()..inner.len() - close.len()];
        return ByteEdit {
            selection: target.start..target.start + unwrapped.len(),
            range: target,
            replacement: unwrapped.to_owned(),
        };
    }
    let start = target.start + open.len();
    ByteEdit { selection: start..start + inner.len(), replacement: format!("{open}{inner}{close}"), range: target }
}

/// Markdown ignores markers next to spaces, so ` palabra ` becomes `**palabra**`.
fn trimming_whitespace(text: &str, selection: Range<usize>) -> Range<usize> {
    let selected = &text[selection.clone()];
    let start = selection.start + (selected.len() - selected.trim_start().len());
    let end = (selection.end - (selected.len() - selected.trim_end().len())).max(start);
    start..end
}

fn markers_around(open: &str, close: &str, text: &str, target: &Range<usize>) -> Option<Range<usize>> {
    let before = target.start.checked_sub(open.len())?..target.start;
    let after = target.end..target.end + close.len();
    if text.get(before.clone())? != open || text.get(after.clone())? != close {
        return None;
    }
    // `_` inside `__bold__` belongs to the bold markers. A single marker only counts when
    // its run is odd: one marker on its own, or three for bold and italic together.
    if open == close && open.len() == 1 {
        let marker = open.as_bytes()[0];
        let run = text.as_bytes()[..before.end].iter().rev().take_while(|&&b| b == marker).count();
        if run % 2 == 0 {
            return None;
        }
    }
    Some(before.start..after.end)
}

// -------------------------------------------------------------------------- lines

/// Tools that act on every line the selection touches.
struct LineEditor<'a> {
    text: &'a str,
    selection: Range<usize>,
}

/// A rewritten line, with the lengths of the prefix it replaced and of the one it wrote,
/// which is how the caret stays on the same character.
struct LineResult {
    line: String,
    old_prefix: usize,
    new_prefix: usize,
}

impl LineEditor<'_> {
    fn line_range(&self) -> Range<usize> {
        // A selection that ends right after a line break doesn't reach into the next line.
        let mut covered = self.selection.clone();
        if !covered.is_empty() && self.text.as_bytes()[covered.end - 1] == b'\n' {
            covered.end -= 1;
        }
        let first = line_range(self.text, covered.start);
        let last = line_range(self.text, if covered.is_empty() { covered.start } else { covered.end - 1 });
        first.start..last.end
    }

    /// The lines without their final line break, which stays where it is.
    fn block(&self) -> (Range<usize>, Vec<&str>) {
        let mut range = self.line_range();
        if range.end > range.start && self.text.as_bytes()[range.end - 1] == b'\n' {
            range.end -= 1;
        }
        let lines = self.text[range.clone()].split('\n').collect();
        (range, lines)
    }

    fn heading(&self, level: u8) -> ByteEdit {
        let (_, lines) = self.block();
        let levels: Vec<usize> = lines
            .iter()
            .filter(|l| is_content(l))
            .map(|l| HEADING_LEVEL.captures(l).ok().flatten().map_or(0, |c| c.get(1).unwrap().as_str().len()))
            .collect();
        let applied = level > 0 && !levels.is_empty() && levels.iter().all(|&l| l == level as usize);
        let prefix = if applied || level == 0 { String::new() } else { format!("{} ", "#".repeat(level as usize)) };
        self.replacing_prefixes(&HEADING_PREFIX, |_| prefix.clone())
    }

    fn quote(&self) -> ByteEdit {
        let (_, lines) = self.block();
        let quoted = lines.iter().filter(|l| is_content(l)).all(|l| matches(&QUOTE_PREFIX, l));
        if quoted && lines.iter().any(|l| is_content(l)) {
            return self.replacing_prefixes(&QUOTE_PREFIX, |_| String::new());
        }
        // A single empty line is where the person starts typing, so it gets the space too.
        let single = lines.len() == 1;
        self.replacing_prefixes(&EMPTY, |line| if is_content(line) || single { "> ".into() } else { ">".into() })
    }

    fn bullet_list(&self) -> ByteEdit {
        self.list(&BULLET_PREFIX, |_| "- ".into())
    }

    fn numbered_list(&self) -> ByteEdit {
        self.list(&NUMBER_PREFIX, |n| format!("{n}. "))
    }

    fn list(&self, same: &Regex, marker: impl Fn(usize) -> String) -> ByteEdit {
        let (_, lines) = self.block();
        let applied = lines.iter().filter(|l| is_content(l)).all(|l| matches(same, l));
        if applied && lines.iter().any(|l| is_content(l)) {
            return self.replacing_list_markers(|_| String::new());
        }
        self.replacing_list_markers(marker)
    }

    fn code_block(&self) -> ByteEdit {
        let (range, lines) = self.block();
        if lines.len() >= 2 && matches(&FENCE, lines[0]) && matches(&FENCE, lines[lines.len() - 1]) {
            let inner = lines[1..lines.len() - 1].join("\n");
            return ByteEdit { selection: range.start..range.start + inner.len(), range, replacement: inner };
        }
        let content = lines.join("\n");
        let start = range.start + 4;
        ByteEdit { selection: start..start + content.len(), replacement: format!("```\n{content}\n```"), range }
    }

    /// Replaces each line's list marker, numbering the lines with content from 1.
    fn replacing_list_markers(&self, marker: impl Fn(usize) -> String) -> ByteEdit {
        let single = self.block().1.len() == 1;
        let mut ordinal = 0;
        self.transforming_lines(|line| {
            // Several lines at once leave the empty ones alone, but a single empty line is
            // where the person starts typing the first item.
            if !is_content(line) && !single {
                return LineResult { line: line.to_owned(), old_prefix: 0, new_prefix: 0 };
            }
            ordinal += 1;
            let (indentation, old_prefix) = match LIST_PREFIX.captures(line).ok().flatten() {
                Some(caps) => (caps.get(1).unwrap().as_str(), caps.get(0).unwrap().end()),
                None => {
                    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                    (&line[..indent], indent)
                }
            };
            let new = format!("{indentation}{}", marker(ordinal));
            LineResult { line: format!("{new}{}", &line[old_prefix..]), old_prefix, new_prefix: new.len() }
        })
    }

    fn replacing_prefixes(&self, pattern: &Regex, new_prefix: impl Fn(&str) -> String) -> ByteEdit {
        let single = self.block().1.len() == 1;
        self.transforming_lines(|line| {
            // Several lines at once leave the empty ones alone, but a single empty line is
            // where the person wants to start typing.
            let prefix = new_prefix(line);
            if !single && !is_content(line) && prefix != ">" {
                return LineResult { line: line.to_owned(), old_prefix: 0, new_prefix: 0 };
            }
            let old_prefix = prefix_len(pattern, line);
            LineResult { line: format!("{prefix}{}", &line[old_prefix..]), old_prefix, new_prefix: prefix.len() }
        })
    }

    fn transforming_lines(&self, transform: impl FnMut(&str) -> LineResult) -> ByteEdit {
        let (range, lines) = self.block();
        let results: Vec<LineResult> = lines.into_iter().map(transform).collect();
        let replacement = results.iter().map(|r| r.line.as_str()).collect::<Vec<_>>().join("\n");
        if !self.selection.is_empty() {
            return ByteEdit { selection: range.start..range.start + replacement.len(), range, replacement };
        }
        let first = &results[0];
        let column = (self.selection.start - range.start) as isize;
        let column = first.new_prefix.max((column - first.old_prefix as isize + first.new_prefix as isize).max(0) as usize);
        let caret = range.start + column.min(replacement.len());
        ByteEdit { range, replacement, selection: caret..caret }
    }
}

// ------------------------------------------------------------------------- blocks

fn table(dimensions: &TableDimensions) -> String {
    let row = |cells: Vec<&str>| format!("| {} |", cells.join(" | "));
    let mut lines = vec![
        row(dimensions.headers.iter().map(String::as_str).collect()),
        row(vec!["---"; dimensions.columns]),
    ];
    lines.extend((1..dimensions.rows).map(|_| row(vec![""; dimensions.columns])));
    lines.join("\n")
}

/// Inserts a block of its own, with the blank lines Markdown needs around it. Without the
/// one before, `---` under a paragraph would turn that paragraph into a heading.
fn insert_block(text: &str, selection: Range<usize>, block: &str, placeholder: Option<&str>) -> ByteEdit {
    let content_at = |at: usize| content_range(text, line_range(text, at));
    let is_blank = |r: Range<usize>| !is_content(&text[r]);
    let line = content_at(selection.end);
    let (insertion, prefix) = if is_blank(line.clone()) {
        let after_text = line.start > 0 && !is_blank(content_at(line.start - 1));
        (line.clone(), if after_text { "\n" } else { "" })
    } else {
        (line.end..line.end, "\n\n")
    };
    let next_has_content = line.end < text.len() && !is_blank(content_at(line.end + 1));
    let suffix = if next_has_content { "\n" } else { "" };
    let start = insertion.start + prefix.len();
    let selection = match placeholder.and_then(|p| block.find(p).map(|i| (i, p.len()))) {
        Some((offset, len)) => start + offset..start + offset + len,
        None => start + block.len()..start + block.len(),
    };
    ByteEdit { range: insertion, replacement: format!("{prefix}{block}{suffix}"), selection }
}

// -------------------------------------------------------------------------- links

fn link(text: &str, destination: &str, selection: Range<usize>) -> ByteEdit {
    let trimmed = text.trim();
    let label = if trimmed.is_empty() { destination } else { trimmed };
    let label = label.replace('[', "\\[").replace(']', "\\]");
    let target = if destination.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        format!("<{destination}>")
    } else {
        destination.to_owned()
    };
    let replacement = format!("[{label}]({target})");
    let end = selection.start + replacement.len();
    ByteEdit { range: selection, replacement, selection: end..end }
}

/// What a person types as a link destination, made into one Markdown can use. `None` for
/// empty input. Adds `https://` to bare domains and `mailto:` to bare addresses, and leaves
/// anchors, paths and anything with a scheme as they are.
pub fn normalized_link(input: &str) -> Option<String> {
    static SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z][A-Za-z0-9+.\-]*:"));
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if matches(&SCHEME, trimmed) || trimmed.starts_with(['#', '/', '.']) {
        return Some(trimmed.to_owned());
    }
    if trimmed.contains('@') && !trimmed.contains('/') {
        return Some(format!("mailto:{trimmed}"));
    }
    Some(if trimmed.contains('.') { format!("https://{trimmed}") } else { trimmed.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::FormatCommand::*;
    use super::*;
    use crate::markdown::marks::{apply, unmark};

    fn format(command: FormatCommand, marked: &str) -> String {
        let (text, selection) = unmark(marked);
        apply(&text, &edit(&command, &text, selection))
    }

    fn standard_table() -> FormatCommand {
        Table(TableDimensions::new(2, 2, |n| format!("Column {n}")).unwrap())
    }

    // ---- inline

    #[test]
    fn bold_wraps_and_unwraps() {
        assert_eq!(format(Bold, "Hola «mundo»"), "Hola **«mundo»**");
        assert_eq!(format(Bold, "Hola **«mundo»**"), "Hola «mundo»");
        assert_eq!(format(Bold, "Hola «**mundo**»"), "Hola «mundo»");
    }

    #[test]
    fn bold_without_selection_leaves_caret_between_markers() {
        assert_eq!(format(Bold, "Hola ‸"), "Hola **‸**");
    }

    #[test]
    fn surrounding_spaces_stay_outside_markers() {
        assert_eq!(format(Bold, "Hola« mundo »adiós"), "Hola **«mundo»** adiós");
    }

    #[test]
    fn italic_inside_bold_keeps_bold() {
        assert_eq!(format(Italic, "**«negrita»**"), "**_«negrita»_**");
        assert_eq!(format(Italic, "_«uno»_"), "«uno»");
    }

    #[test]
    fn single_marker_inside_double_is_not_removed() {
        assert_eq!(format(Italic, "__«a»__"), "___«a»___");
    }

    #[test]
    fn other_inline_styles() {
        assert_eq!(format(Underline, "«a»"), "<u>«a»</u>");
        assert_eq!(format(Underline, "<u>«a»</u>"), "«a»");
        assert_eq!(format(Strikethrough, "«a»"), "~~«a»~~");
        assert_eq!(format(InlineCode, "«a»"), "`«a»`");
    }

    #[test]
    fn offsets_count_characters() {
        assert_eq!(format(Bold, "🌮 «taco»"), "🌮 **«taco»**");
        assert_eq!(format(Heading(1), "ñandú‸"), "# ñandú‸");
    }

    // ---- headings

    #[test]
    fn headings() {
        assert_eq!(format(Heading(2), "Títul‸o"), "## Títul‸o");
        assert_eq!(format(Heading(3), "# Tít‸ulo"), "### Tít‸ulo");
        assert_eq!(format(Heading(2), "## Tít‸ulo"), "Tít‸ulo");
        assert_eq!(format(Heading(0), "### a‸"), "a‸");
        assert_eq!(format(Heading(1), "‸"), "# ‸");
    }

    // ---- quotes and lists

    #[test]
    fn quotes() {
        assert_eq!(format(Quote, "«uno\ndos»"), "«> uno\n> dos»");
        assert_eq!(format(Quote, "«> uno\n> dos»"), "«uno\ndos»");
        assert_eq!(format(Quote, "«uno\n\ndos»"), "«> uno\n>\n> dos»");
    }

    #[test]
    fn lists() {
        assert_eq!(format(BulletList, "«uno\n\ndos»"), "«- uno\n\n- dos»");
        assert_eq!(format(NumberedList, "«- uno\n- dos»"), "«1. uno\n2. dos»");
        assert_eq!(format(NumberedList, "«1. uno\n2. dos»"), "«uno\ndos»");
        assert_eq!(format(BulletList, "«  uno»"), "«  - uno»");
    }

    #[test]
    fn lists_and_quote_start_on_an_empty_line() {
        assert_eq!(format(BulletList, "‸"), "- ‸");
        assert_eq!(format(NumberedList, "‸"), "1. ‸");
        assert_eq!(format(Quote, "‸"), "> ‸");
        assert_eq!(format(BulletList, "texto\n\n‸"), "texto\n\n- ‸");
    }

    #[test]
    fn empty_item_toggles_back_off() {
        assert_eq!(format(BulletList, "- ‸"), "‸");
    }

    #[test]
    fn selection_ending_at_line_start_leaves_next_line_alone() {
        assert_eq!(format(BulletList, "«uno\n»dos"), "«- uno»\ndos");
    }

    // ---- blocks

    #[test]
    fn code_blocks() {
        assert_eq!(format(CodeBlock, "«let a = 1»"), "```\n«let a = 1»\n```");
        assert_eq!(format(CodeBlock, "«```\nlet a\n```»"), "«let a»");
        assert_eq!(format(CodeBlock, "‸"), "```\n‸\n```");
    }

    #[test]
    fn rules_get_blank_lines() {
        assert_eq!(format(HorizontalRule, "Texto‸"), "Texto\n\n---‸");
        assert_eq!(format(HorizontalRule, "Texto\n‸"), "Texto\n\n---‸");
        assert_eq!(format(HorizontalRule, "‸\nTexto"), "---‸\n\nTexto");
    }

    #[test]
    fn table_selects_first_header() {
        assert_eq!(format(standard_table(), "‸"), "| «Column 1» | Column 2 |\n| --- | --- |\n|  |  |");
    }

    #[test]
    fn table_dimensions_are_limited() {
        assert!(TableDimensions::new(0, 2, |n| n.to_string()).is_none());
        assert!(TableDimensions::new(21, 2, |n| n.to_string()).is_none());
        assert!(TableDimensions::new(20, 101, |n| n.to_string()).is_none());
        assert!(TableDimensions::new(20, 100, |n| n.to_string()).is_some());
        let one_row = Table(TableDimensions::new(1, 1, |_| "A".into()).unwrap());
        assert_eq!(format(one_row, "‸"), "| «A» |\n| --- |");
    }

    // ---- links

    #[test]
    fn links() {
        let link = |t: &str, d: &str| Link { text: t.into(), destination: d.into() };
        assert_eq!(format(link("Amoxtli", "https://amoxtli.dev"), "Visita «Amoxtli»"), "Visita [Amoxtli](https://amoxtli.dev)‸");
        assert_eq!(format(link("a [b]", "mi archivo.md"), "‸"), "[a \\[b\\]](<mi archivo.md>)‸");
        assert_eq!(format(link(" ", "https://a.b"), "‸"), "[https://a.b](https://a.b)‸");
    }

    #[test]
    fn link_input() {
        assert_eq!(normalized_link("amoxtli.dev").as_deref(), Some("https://amoxtli.dev"));
        assert_eq!(normalized_link(" http://a.b/c ").as_deref(), Some("http://a.b/c"));
        assert_eq!(normalized_link("hola@amoxtli.dev").as_deref(), Some("mailto:hola@amoxtli.dev"));
        assert_eq!(normalized_link("#intro").as_deref(), Some("#intro"));
        assert_eq!(normalized_link("./notas.md").as_deref(), Some("./notas.md"));
        assert_eq!(normalized_link("notas").as_deref(), Some("notas"));
        assert_eq!(normalized_link("  \n"), None);
    }
}
