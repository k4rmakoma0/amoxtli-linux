//! Writes the formatted editor's paragraphs back as Markdown. The output is canonical, not
//! the original source: `-` for bullets, `**` and `*` for emphasis, fenced code. What it
//! guarantees, and the tests check, is that parsing the result gives back the same document.

use super::model::*;
use std::collections::HashMap;
use std::sync::LazyLock;

pub fn markdown(doc: &Document) -> String {
    let mut s = Serializer::default();
    let paragraphs = &doc.paragraphs;
    let mut index = 0;
    while index < paragraphs.len() {
        let end = unit_end(index, paragraphs);
        s.write(&paragraphs[index..end]);
        index = end;
    }
    let mut out = String::new();
    if let Some(front) = &doc.front_matter {
        out.push_str(front);
        out.push('\n');
        if !s.lines.is_empty() {
            out.push('\n');
        }
    }
    if !s.lines.is_empty() {
        out.push_str(&s.lines.join("\n"));
        out.push('\n');
    }
    out
}

/// Code lines, raw lines and table cells are written together as one block.
fn unit_end(start: usize, paragraphs: &[Paragraph]) -> usize {
    let first = &paragraphs[start];
    let mut end = start + 1;
    while end < paragraphs.len() && belong_together(first, &paragraphs[end]) {
        end += 1;
    }
    end
}

fn belong_together(a: &Paragraph, b: &Paragraph) -> bool {
    if a.kind.containers != b.kind.containers {
        return false;
    }
    match (&a.kind.block, &b.kind.block) {
        (Block::Code { id: x, .. }, Block::Code { id: y, .. }) => x == y,
        (Block::Raw { id: x }, Block::Raw { id: y }) => x == y,
        (Block::TableCell(x), Block::TableCell(y)) => x.table_id == y.table_id,
        _ => false,
    }
}

fn trim_trailing_spaces(line: &str) -> String {
    line.trim_end_matches(' ').to_owned()
}

#[derive(Default)]
struct Serializer {
    lines: Vec<String>,
    previous: Option<Paragraph>,
    items_seen: HashMap<u32, u64>,
    marker_widths: HashMap<u32, usize>,
    bullet_by_list: HashMap<u32, char>,
}

impl Serializer {
    fn write(&mut self, unit: &[Paragraph]) {
        let (Some(first), Some(last)) = (unit.first(), unit.last()) else { return };
        // An empty top-level paragraph is only a blank line, which Markdown already puts
        // between blocks.
        if first.kind.block == Block::Paragraph
            && first.kind.containers.is_empty()
            && first.runs.iter().all(|r| r.text.chars().all(char::is_whitespace))
        {
            return;
        }
        if let Some(previous) = self.previous.clone() {
            self.write_separator(&previous, first);
        }
        let (first_prefix, continuation) = self.prefixes(first);
        for (i, line) in content(unit).into_iter().enumerate() {
            let prefixed = format!("{}{line}", if i == 0 { &first_prefix } else { &continuation });
            self.lines.push(if line.is_empty() { trim_trailing_spaces(&prefixed) } else { prefixed });
        }
        self.previous = Some(last.clone());
    }

    /// The prefix of the paragraph's first line and of the lines after it. A list item's
    /// marker only goes on the first line of its first paragraph; the rest is indented to match.
    fn prefixes(&mut self, paragraph: &Paragraph) -> (String, String) {
        let mut first = String::new();
        let mut continuation = String::new();
        for (depth, container) in paragraph.kind.containers.iter().enumerate() {
            match container {
                Container::Quote { alert, .. } => {
                    let continuing = self.previous.as_ref().is_some_and(|p| p.kind.containers.get(depth) == Some(container));
                    first.push_str("> ");
                    continuation.push_str("> ");
                    if let (Some(alert), false) = (alert, continuing) {
                        // the alert's marker is a line of its own at the top of the quote
                        first = format!("{first}[!{}]\n{first}", alert.tag());
                    }
                }
                Container::ListItem(item) => {
                    let continuing = self.previous.as_ref().is_some_and(|p| p.kind.containers.get(depth) == Some(container));
                    if continuing {
                        let indent = " ".repeat(self.marker_widths.get(&item.item_id).copied().unwrap_or(2));
                        first.push_str(&indent);
                        continuation.push_str(&indent);
                    } else {
                        let marker = self.new_item_marker(item, depth);
                        let width = marker.chars().count();
                        first.push_str(&marker);
                        if let Some(done) = item.task {
                            first.push_str(if done { "[x] " } else { "[ ] " });
                        }
                        continuation.push_str(&" ".repeat(width));
                    }
                }
            }
        }
        (first, continuation)
    }

    fn new_item_marker(&mut self, item: &ListItem, depth: usize) -> String {
        // Ids are only unique among neighbors: a list that doesn't continue the one above
        // starts its count and its bullet afresh.
        let continues = self
            .previous
            .as_ref()
            .and_then(|p| p.kind.containers.get(depth))
            .and_then(Container::list_item)
            .is_some_and(|other| other.list_id == item.list_id);
        if !continues {
            self.items_seen.remove(&item.list_id);
            self.bullet_by_list.remove(&item.list_id);
        }
        let bullet = self.bullet(item, depth);
        let marker = if item.ordered {
            let index = self.items_seen.entry(item.list_id).or_insert(0);
            let number = item.start + *index;
            *index += 1;
            format!("{number}{bullet} ")
        } else {
            format!("{bullet} ")
        };
        self.marker_widths.insert(item.item_id, marker.chars().count());
        marker
    }

    /// Two different lists one after the other would merge into one if they used the same
    /// marker, so the second one switches to the alternative.
    fn bullet(&mut self, item: &ListItem, depth: usize) -> char {
        if let Some(known) = self.bullet_by_list.get(&item.list_id) {
            return *known;
        }
        let (standard, alternative) = if item.ordered { ('.', ')') } else { ('-', '*') };
        let mut chosen = standard;
        if let Some(Container::ListItem(neighbor)) = self.previous.as_ref().and_then(|p| p.kind.containers.get(depth)) {
            if neighbor.list_id != item.list_id
                && neighbor.ordered == item.ordered
                && self.bullet_by_list.get(&neighbor.list_id) == Some(&standard)
            {
                chosen = alternative;
            }
        }
        self.bullet_by_list.insert(item.list_id, chosen);
        chosen
    }

    /// List items follow each other directly; anything else gets a blank line, which keeps
    /// the quotes and list items it is inside.
    fn write_separator(&mut self, previous: &Paragraph, next: &Paragraph) {
        let (pc, nc) = (&previous.kind.containers, &next.kind.containers);
        let shared = pc.iter().zip(nc).take_while(|(a, b)| a == b).count();
        if let Some(Container::ListItem(item)) = nc.get(shared) {
            if let Some(Container::ListItem(other)) = pc.get(shared) {
                if other.list_id == item.list_id {
                    return;
                }
            }
            let starts_sublist = shared == pc.len() && shared > 0 && (!item.ordered || item.start == 1);
            if starts_sublist && previous.kind.block.is_text() {
                return;
            }
        }
        let mut blank = String::new();
        for container in &nc[..shared] {
            match container {
                Container::Quote { .. } => blank.push_str("> "),
                Container::ListItem(item) => {
                    blank.push_str(&" ".repeat(self.marker_widths.get(&item.item_id).copied().unwrap_or(2)))
                }
            }
        }
        self.lines.push(trim_trailing_spaces(&blank));
    }
}

fn content(unit: &[Paragraph]) -> Vec<String> {
    let first = &unit[0];
    match &first.kind.block {
        Block::Paragraph => inline(&first.runs, false)
            .split('\n')
            .enumerate()
            .map(|(i, line)| escaped_line_start(if i == 0 { line } else { line.trim_start_matches(' ') }))
            .collect(),
        Block::Heading { level, attributes } => {
            let runs: Vec<InlineRun> =
                first.runs.iter().map(|r| InlineRun { text: r.text.replace('\n', " "), ..r.clone() }).collect();
            let mut line = format!("{} {}", "#".repeat(*level as usize), escaping_closing_hashes(&inline(&runs, false)));
            if let Some(attributes) = attributes {
                line = format!("{} {attributes}", line.trim_end());
            }
            vec![trim_trailing_spaces(&line)]
        }
        Block::Code { info, .. } => {
            let code: Vec<String> = unit.iter().map(Paragraph::plain_text).collect();
            let longest = code.iter().map(|c| longest_run(c, '`')).max().unwrap_or(0);
            let fence = "`".repeat((longest + 1).max(3));
            let mut lines = vec![format!("{fence}{}", info.as_deref().unwrap_or(""))];
            lines.extend(code);
            lines.push(fence);
            lines
        }
        Block::Raw { .. } => unit.iter().map(Paragraph::plain_text).collect(),
        Block::ThematicBreak => vec!["---".into()],
        Block::Image(image) => {
            vec![self::image(&image.alt, &image.source, &image.title)]
        }
        Block::TableCell(_) => table_lines(unit),
    }
}

// ------------------------------------------------------------------ line starts

/// Text at the start of a line that Markdown would read as syntax.
fn escaped_line_start(line: &str) -> String {
    static SYNTAX: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        fancy_regex::Regex::new(r"^(?:#{1,6}(?:\s|$)|>|[-+*](?:\s|$)|(?:=+|-+)\s*$|```|~~~)").unwrap()
    });
    static NUMBER: LazyLock<fancy_regex::Regex> =
        LazyLock::new(|| fancy_regex::Regex::new(r"^\d{1,9}(?=[.)](?:\s|$))").unwrap());
    if SYNTAX.is_match(line).unwrap_or(false) {
        return format!("\\{line}");
    }
    if let Ok(Some(m)) = NUMBER.find(line) {
        return format!("{}\\{}", &line[..m.end()], &line[m.end()..]);
    }
    line.to_owned()
}

/// A heading's text ending in `#` would lose it as a closing sequence.
fn escaping_closing_hashes(text: &str) -> String {
    let trimmed = text.trim_end_matches('#');
    if trimmed.len() == text.len() {
        return text.to_owned();
    }
    format!("{trimmed}\\{}", &text[trimmed.len()..])
}

// ------------------------------------------------------------------------ tables

fn table_lines(cells: &[Paragraph]) -> Vec<String> {
    let Some(alignments) = cells[0].kind.table_cell().map(|c| c.alignments.clone()) else { return vec![] };
    if alignments.is_empty() {
        return vec![];
    }
    let rows = cells.iter().filter_map(|p| p.kind.table_cell().map(|c| c.row)).max().unwrap_or(0) + 1;
    let mut grid = vec![vec![String::new(); alignments.len()]; rows];
    for p in cells {
        let Some(cell) = p.kind.table_cell() else { continue };
        if cell.column < alignments.len() {
            grid[cell.row][cell.column] = inline(&p.runs, true).replace("\\\n", " ");
        }
    }
    let row = |cells: &[String]| format!("| {} |", cells.join(" | "));
    let delimiter: Vec<String> = alignments
        .iter()
        .map(|a| match a {
            Alignment::None => "---",
            Alignment::Left => ":---",
            Alignment::Center => ":---:",
            Alignment::Right => "---:",
        })
        .map(String::from)
        .collect();
    let mut lines = vec![row(&grid[0]), row(&delimiter)];
    lines.extend(grid[1..].iter().map(|r| row(r)));
    lines
}

// ------------------------------------------------------------------------ inline

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Underline,
    Strikethrough,
    Strong,
    Emphasis,
}

impl Marker {
    const ALL: [Marker; 4] = [Marker::Underline, Marker::Strikethrough, Marker::Strong, Marker::Emphasis];

    fn style(self) -> InlineStyle {
        match self {
            Marker::Underline => InlineStyle::UNDERLINE,
            Marker::Strikethrough => InlineStyle::STRIKETHROUGH,
            Marker::Strong => InlineStyle::STRONG,
            Marker::Emphasis => InlineStyle::EMPHASIS,
        }
    }

    fn open(self) -> &'static str {
        match self {
            Marker::Underline => "<u>",
            Marker::Strikethrough => "~~",
            Marker::Strong => "**",
            Marker::Emphasis => "*",
        }
    }

    fn close(self) -> &'static str {
        if self == Marker::Underline { "</u>" } else { self.open() }
    }
}

pub fn inline(runs: &[InlineRun], escaping_pipes: bool) -> String {
    let runs: Vec<&InlineRun> = runs.iter().filter(|r| !r.text.is_empty()).collect();
    let characters: Vec<char> = runs.iter().filter(|r| !r.style.contains(InlineStyle::RAW)).flat_map(|r| r.text.chars()).collect();
    let mut w = InlineWriter { escaper: Escaper::new(characters, escaping_pipes), out: String::new(), open: vec![], offset: 0 };
    let mut i = 0;
    while i < runs.len() {
        let link = runs[i].link.clone();
        let mut j = i;
        while j < runs.len() && runs[j].link == link {
            j += 1;
        }
        match &link {
            None => w.runs(&runs, i..j, InlineStyle::NONE),
            Some(link) => {
                // Markers every run of the link has stay open around it, so a link inside
                // bold text doesn't split the bold in two.
                let common = runs[i..j].iter().fold(InlineStyle(0xff), |acc, r| acc.intersection(r.style)).without(InlineStyle::CODE);
                w.set_markers(common);
                let floor = w.open.len();
                let shown: String = runs[i..j].iter().map(|r| r.text.as_str()).collect();
                let plain = runs[i..j].iter().all(|r| r.style.without(InlineStyle::CODE) == common && !r.style.contains(InlineStyle::CODE));
                if link.title.is_empty() && plain && shown == link.destination && is_autolink(&link.destination) {
                    w.out.push_str(&format!("<{}>", link.destination));
                    w.offset += shown.chars().count();
                } else {
                    w.out.push('[');
                    w.runs(&runs, i..j, common);
                    w.close_from(floor);
                    w.out.push_str(&format!("]({}{})", destination(&link.destination), title(&link.title)));
                }
            }
        }
        i = j;
    }
    w.close_from(0);
    w.out
}

pub fn title(title: &str) -> String {
    if title.is_empty() { String::new() } else { format!(" \"{}\"", title.replace('"', "\\\"")) }
}

/// Markdown for an image, as written inline.
pub fn image(alt: &str, source: &str, image_title: &str) -> String {
    format!("![{}]({}{})", escaped_label(alt), destination(source), title(image_title))
}

struct InlineWriter {
    escaper: Escaper,
    out: String,
    open: Vec<Marker>,
    offset: usize,
}

impl InlineWriter {
    fn close_from(&mut self, from: usize) {
        for m in self.open[from.min(self.open.len())..].iter().rev() {
            self.out.push_str(m.close());
        }
        self.open.truncate(from);
    }

    /// Closes the open markers a style doesn't have and opens the ones it adds.
    fn set_markers(&mut self, style: InlineStyle) {
        let wanted: Vec<Marker> = Marker::ALL.into_iter().filter(|m| style.contains(m.style())).collect();
        if let Some(first) = self.open.iter().position(|m| !wanted.contains(m)) {
            self.close_from(first);
        }
        for m in wanted {
            if !self.open.contains(&m) {
                self.out.push_str(m.open());
                self.open.push(m);
            }
        }
    }

    /// Opens and closes markers as the style changes from run to run. Spaces at the edges of
    /// a styled run go outside its markers, since `** bold**` isn't bold in Markdown; they keep
    /// the styles they share with the run beside them, and `keep`, which stay open across.
    fn runs(&mut self, runs: &[&InlineRun], range: std::ops::Range<usize>, keep: InlineStyle) {
        for k in range {
            let run = runs[k];
            if run.style.contains(InlineStyle::RAW) {
                self.close_from(0);
                self.out.push_str(&run.text);
                continue;
            }
            let before = k.checked_sub(1).map_or(InlineStyle::NONE, |b| runs[b].style);
            let after = runs.get(k + 1).map_or(InlineStyle::NONE, |a| a.style);
            for piece in pieces(run, before, after) {
                self.set_markers(piece.style.union(keep).without(InlineStyle::CODE));
                let len = piece.text.chars().count();
                if piece.style.contains(InlineStyle::CODE) {
                    self.out.push_str(&code_span(&piece.text));
                } else {
                    self.out.push_str(&self.escaper.escaped(self.offset..self.offset + len));
                }
                self.offset += len;
            }
        }
    }
}

/// A URL or address written as its own text, as `<https://a.b>`.
fn is_autolink(text: &str) -> bool {
    static AUTO: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        fancy_regex::Regex::new(r"^(?:[A-Za-z][A-Za-z0-9+.\-]{1,31}:[^\s<>]*|[A-Za-z0-9.!#$%&'*+/=?^_`{|}~\-]+@[A-Za-z0-9](?:[A-Za-z0-9\-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9\-]*[A-Za-z0-9])?)*)$").unwrap()
    });
    AUTO.is_match(text).unwrap_or(false)
}

pub fn destination(url: &str) -> String {
    if url.is_empty() {
        return "<>".into();
    }
    if url.chars().any(|c| c.is_whitespace() || "()<>".contains(c)) {
        format!("<{}>", url.replace('>', "%3E").replace('<', "%3C"))
    } else {
        url.to_owned()
    }
}

pub fn escaped_label(text: &str) -> String {
    text.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

fn longest_run(text: &str, c: char) -> usize {
    let (mut longest, mut current) = (0, 0);
    for ch in text.chars() {
        current = if ch == c { current + 1 } else { 0 };
        longest = longest.max(current);
    }
    longest
}

/// The spaces keep the styles they share with the run beside them, which stay open across.
fn pieces(run: &InlineRun, before: InlineStyle, after: InlineStyle) -> Vec<InlineRun> {
    if run.style.is_empty() || run.style.contains(InlineStyle::CODE) {
        return vec![run.clone()];
    }
    let text = run.text.as_str();
    let rest = text.trim_start();
    let leading = &text[..text.len() - rest.len()];
    let core = rest.trim_end();
    let trailing = &rest[core.len()..];
    [
        (leading, run.style.intersection(before).without(InlineStyle::CODE)),
        (core, run.style),
        (trailing, run.style.intersection(after).without(InlineStyle::CODE)),
    ]
    .into_iter()
    .filter(|(t, _)| !t.is_empty())
    .map(|(t, style)| InlineRun { text: t.to_owned(), style, link: run.link.clone() })
    .collect()
}

fn code_span(code: &str) -> String {
    let fence = "`".repeat(longest_run(code, '`') + 1);
    let padding = if code.starts_with('`') || code.ends_with('`') || (code.starts_with(' ') && code.ends_with(' ') && code.trim() != "") {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{code}{padding}{fence}")
}

/// Escapes the characters of a paragraph that Markdown would otherwise read as syntax. It
/// sees the whole paragraph, since whether `*` or `[` is syntax depends on what surrounds it.
struct Escaper {
    characters: Vec<char>,
    escapes_pipes: bool,
    has_link_syntax: bool,
}

impl Escaper {
    fn new(characters: Vec<char>, escapes_pipes: bool) -> Self {
        let text: String = characters.iter().collect();
        let has_link_syntax = text.contains("](") || text.contains("][") || text.contains("[^");
        Self { characters, escapes_pipes, has_link_syntax }
    }

    fn escaped(&self, range: std::ops::Range<usize>) -> String {
        range.map(|i| self.escaped_at(i)).collect()
    }

    fn escaped_at(&self, index: usize) -> String {
        let c = self.characters[index];
        let before = index.checked_sub(1).map(|i| self.characters[i]);
        let after = self.characters.get(index + 1).copied();
        let space_or_edge = |c: Option<char>| c.is_none_or(char::is_whitespace);
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        match c {
            '\\' | '`' => format!("\\{c}"),
            '\n' => "\\\n".into(),
            '*' | '~' => {
                if space_or_edge(before) && space_or_edge(after) {
                    c.to_string()
                } else {
                    format!("\\{c}")
                }
            }
            '_' => {
                if (word(before) && word(after)) || (space_or_edge(before) && space_or_edge(after)) {
                    "_".into()
                } else {
                    "\\_".into()
                }
            }
            '[' | ']' => {
                if self.has_link_syntax { format!("\\{c}") } else { c.to_string() }
            }
            '!' if after == Some('[') && self.has_link_syntax => "\\!".into(),
            '<' => {
                if after.is_some_and(|a| a.is_alphabetic() || "/!?".contains(a)) { "\\<".into() } else { "<".into() }
            }
            '&' => {
                if self.starts_entity(index) { "\\&".into() } else { "&".into() }
            }
            '|' if self.escapes_pipes => "\\|".into(),
            _ => c.to_string(),
        }
    }

    fn starts_entity(&self, index: usize) -> bool {
        let rest: String = self.characters[index + 1..].iter().take(32).collect();
        static ENTITY: LazyLock<fancy_regex::Regex> = LazyLock::new(|| fancy_regex::Regex::new(r"^#?[A-Za-z0-9]+;").unwrap());
        ENTITY.is_match(&rest).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::*;

    /// The guarantee: writing a document and reading it back gives the same document.
    fn round_trip(source: &str) -> String {
        let doc = parse(source);
        let written = markdown(&doc);
        let again = parse(&written);
        assert_eq!(again, doc, "\n--- source\n{source}\n--- written\n{written}");
        // and the output is stable
        assert_eq!(markdown(&again), written, "unstable output for\n{source}");
        written
    }

    #[test]
    fn inline_styles() {
        assert_eq!(round_trip("Some **bold**, *italic*, ~~gone~~, `code` and <u>under</u>.\n"),
            "Some **bold**, *italic*, ~~gone~~, `code` and <u>under</u>.\n");
        assert_eq!(round_trip("__strong__ and _em_\n"), "**strong** and *em*\n");
        round_trip("***both*** and **bold *nested* bold**\n");
        round_trip("**bold with `code` inside**\n");
        round_trip("a <u>**strong under**</u> b\n");
    }

    #[test]
    fn spaces_stay_outside_markers() {
        let doc = Document {
            front_matter: None,
            paragraphs: vec![Paragraph::new(Block::Paragraph, vec![], vec![
                InlineRun::plain("a"),
                InlineRun::styled(" b ", InlineStyle::STRONG),
                InlineRun::plain("c"),
            ])],
        };
        assert_eq!(markdown(&doc), "a **b** c\n");
    }

    #[test]
    fn lists() {
        assert_eq!(round_trip("* one\n* two\n  * nested\n\n1. a\n2. b\n"), "- one\n- two\n  - nested\n\n1. a\n2. b\n");
        round_trip("3. three\n4. four\n");
        round_trip("- a\n\n  second paragraph\n- b\n");
        round_trip("- [ ] todo\n- [x] done\n");
        round_trip("1. one\n   - inner\n2. two\n");
        // lists apart restart their numbers, though their ids repeat
        assert_eq!(round_trip("1. a\n2. b\n\ntext\n\n1. c\n2. d\n\nmore\n\n5. e\n"), "1. a\n2. b\n\ntext\n\n1. c\n2. d\n\nmore\n\n5. e\n");
    }

    #[test]
    fn adjacent_lists_keep_apart() {
        let a = Container::ListItem(ListItem { list_id: 1, item_id: 2, ordered: false, start: 1, task: None });
        let b = Container::ListItem(ListItem { list_id: 3, item_id: 4, ordered: false, start: 1, task: None });
        let doc = Document {
            front_matter: None,
            paragraphs: vec![
                Paragraph::new(Block::Paragraph, vec![a], vec![InlineRun::plain("a")]),
                Paragraph::new(Block::Paragraph, vec![b], vec![InlineRun::plain("b")]),
            ],
        };
        let written = markdown(&doc);
        assert_eq!(written, "- a\n\n* b\n");
        assert_eq!(parse(&written).paragraphs.len(), 2);
    }

    #[test]
    fn quotes_and_alerts() {
        round_trip("> quoted **text**\n>\n> - item in quote\n");
        assert_eq!(round_trip("> [!NOTE]\n> Take note.\n"), "> [!NOTE]\n> Take note.\n");
        round_trip("> outer\n>\n> > inner\n");
    }

    #[test]
    fn code_blocks() {
        assert_eq!(round_trip("```rust\nfn main() {}\n\n// end\n```\n"), "```rust\nfn main() {}\n\n// end\n```\n");
        assert_eq!(round_trip("    indented\n"), "```\nindented\n```\n");
        round_trip("````\n```\nfence inside\n```\n````\n");
        round_trip("```py title=\"a\"\nx = 1\n```\n");
        round_trip("- item\n\n  ```\n  code in item\n  ```\n");
    }

    #[test]
    fn tables() {
        let written = round_trip("| A | B |\n|:--|--:|\n| 1 | **2** |\n|  |  |\n");
        assert_eq!(written, "| A | B |\n| :--- | ---: |\n| 1 | **2** |\n|  |  |\n");
        round_trip("| a \\| b | c |\n| --- | :---: |\n| x | y |\n");
        round_trip("> | q |\n> | --- |\n> |  |\n> |  |\n");
    }

    #[test]
    fn empty_table_rows_are_kept() {
        let doc = parse("| A |\n| --- |\n|  |\n|  |\n\ntext\n");
        let cells = doc.paragraphs.iter().filter(|p| p.kind.table_cell().is_some()).count();
        assert_eq!(cells, 3);
        round_trip("| A |\n| --- |\n|  |\n|  |\n\ntext\n");
    }

    #[test]
    fn text_that_looks_like_syntax() {
        for text in ["\\# not a heading", "1\\. not a list", "\\- not a bullet", "\\> not a quote", "a \\*b\\* c",
                     "under\\_score\\_", "snake_case_name", "\\[not\\](a link)", "\\<b> tag", "&amp; entity", "\\`tick\\`",
                     "2 * 3 * 4", "\\---", "\\```"] {
            round_trip(&format!("{text}\n"));
        }
    }

    #[test]
    fn headings() {
        assert_eq!(round_trip("Title\n=====\n\n## Sub ##\n"), "# Title\n\n## Sub\n");
        round_trip("# C\\#\n");
        assert_eq!(round_trip("## Intro {#intro .x}\n"), "## Intro {#intro .x}\n");
    }

    #[test]
    fn links_and_images() {
        round_trip("[a link](https://example.com \"title\") and <https://auto.link>\n");
        round_trip("[spaces](<my file.md>)\n");
        assert_eq!(round_trip("![alt](img.png)\n"), "![alt](img.png)\n");
        round_trip("text with ![inline](i.png) image\n");
        round_trip("[![badge](https://b.svg)](https://ci)\n");
        round_trip("![a](1.png)\n![b](2.png)\n");
    }

    #[test]
    fn hard_breaks() {
        assert_eq!(round_trip("line one  \nline two\n"), "line one\\\nline two\n");
        round_trip("a<br>b\n");
    }

    #[test]
    fn kept_as_written() {
        round_trip("<div align=\"center\">\n  <img src=\"a.png\">\n</div>\n\ntext\n");
        round_trip("Text with a note[^1].\n\n[^1]: The note.\n");
        round_trip("inline <span>html</span> stays\n");
        let written = round_trip("---\ntitle: x\n---\n\n# Doc\n");
        assert!(written.starts_with("---\ntitle: x\n---\n"));
    }

    #[test]
    fn thematic_break() {
        assert_eq!(round_trip("a\n\n***\n\nb\n"), "a\n\n---\n\nb\n");
    }

    #[test]
    fn empty_items_and_quotes() {
        round_trip("- \n- b\n");
        round_trip(">\n");
    }

    #[test]
    fn unicode() {
        round_trip("Ñandú **🌮 taco** _ñ_\n");
    }
}

#[cfg(test)]
mod corpus {
    use super::super::parse::parse;
    use super::markdown;

    fn check(path: &std::path::Path) -> Result<(), String> {
        let Ok(source) = std::fs::read_to_string(path) else { return Ok(()) };
        let doc = parse(&source);
        let written = markdown(&doc);
        let again = parse(&written);
        if again != doc {
            let first = doc.paragraphs.iter().zip(&again.paragraphs).position(|(a, b)| a != b);
            let detail = first.map(|i| format!("\n  read:    {:?}\n  re-read: {:?}", doc.paragraphs[i], again.paragraphs[i])).unwrap_or_default();
            return Err(format!("{}: differs at paragraph {first:?} ({} vs {}){detail}", path.display(), doc.paragraphs.len(), again.paragraphs.len()));
        }
        if markdown(&again) != written {
            return Err(format!("{}: unstable output", path.display()));
        }
        Ok(())
    }

    fn markdown_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules" {
                markdown_files(&path, out);
            } else if name.ends_with(".md") || name.ends_with(".markdown") {
                out.push(path);
            }
        }
    }

    /// Every example in the repository round-trips. `AMOXTLI_CORPUS=dir` checks another
    /// folder of Markdown files too.
    #[test]
    fn examples_round_trip() {
        let mut files = vec![];
        markdown_files(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/examples")), &mut files);
        if let Ok(dir) = std::env::var("AMOXTLI_CORPUS") {
            markdown_files(std::path::Path::new(&dir), &mut files);
        }
        assert!(!files.is_empty());
        let failures: Vec<String> = files.iter().filter_map(|f| check(f).err()).collect();
        assert!(failures.is_empty(), "{} of {} files:\n{}", failures.len(), files.len(), failures.join("\n"));
    }
}
