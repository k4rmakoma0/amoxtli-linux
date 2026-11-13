//! Reads Markdown into the formatted editor's paragraphs (`model`), with pulldown-cmark.
//! Every list, item, quote, code block, raw block and table gets its own id.

use super::model::*;
use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use std::rc::Rc;

/// The syntax the editor understands. No smart punctuation: the editor writes back what it
/// reads, so curly quotes would change the file.
pub fn options() -> Options {
    let mut opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    opts.insert(Options::ENABLE_GFM);
    opts
}

pub fn parse(source: &str) -> Document {
    let mut b = Builder { source, ..Default::default() };
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        b.event(event, range);
    }
    b.close_text();
    b.doc
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    Code,
    Raw,
    Table,
    Quote(usize),
    List(usize),
}

#[derive(Clone)]
enum Open {
    Paragraph,
    /// Text of a tight list item, which has no paragraph events.
    Implicit,
    Heading(u8, Option<String>),
    Cell(TableCell),
}

struct LinkState {
    link: Rc<Link>,
    first_run: usize,
    has_image: bool,
}

struct TableState {
    id: u32,
    alignments: Rc<[Alignment]>,
    row: usize,
    column: usize,
}

#[derive(Default)]
struct Builder<'a> {
    source: &'a str,
    /// How many blocks of each sort so far, to alternate their ids.
    counts: std::collections::HashMap<Slot, u32>,
    doc: Document,
    containers: Vec<Container>,
    /// The paragraph count when each container opened, to give an empty one a paragraph.
    container_starts: Vec<usize>,
    /// The open lists: id, ordered, start, and how many items so far.
    lists: Vec<(u32, bool, u64, u32)>,
    open: Option<Open>,
    runs: Vec<InlineRun>,
    /// Images among `runs`, kept as written unless the paragraph is only images.
    images: Vec<(usize, Image)>,
    emphasis: u32,
    strong: u32,
    strike: u32,
    underline: u32,
    links: Vec<LinkState>,
    image: Option<(Image, Range<usize>)>,
    code: Option<(Option<String>, String)>,
    html: Option<String>,
    table: Option<TableState>,
    /// Inside a footnote definition or front matter, kept as written: events are skipped.
    skip: u32,
}

impl Builder<'_> {
    /// Ids only tell a block from the one beside it (one item from the next, one code block
    /// from the next), so they alternate between two values per sort of block and depth. The
    /// editor keeps a tag per distinct kind, and GTK slows down with every tag: with an id per
    /// item, a long document had thousands.
    fn alternate(&mut self, slot: Slot) -> u32 {
        let count = self.counts.entry(slot).or_insert(0);
        *count += 1;
        let base = match slot {
            Slot::Code => 1,
            Slot::Raw => 3,
            Slot::Table => 5,
            Slot::Quote(depth) => 10 + 2 * depth as u32,
            Slot::List(depth) => 200 + 2 * depth as u32,
        };
        base + *count % 2
    }

    fn push(&mut self, block: Block, runs: Vec<InlineRun>) {
        self.doc.paragraphs.push(Paragraph::new(block, self.containers.clone(), runs));
    }

    fn style(&self) -> InlineStyle {
        let mut s = InlineStyle::NONE;
        if self.emphasis > 0 {
            s = s.union(InlineStyle::EMPHASIS);
        }
        if self.strong > 0 {
            s = s.union(InlineStyle::STRONG);
        }
        if self.strike > 0 {
            s = s.union(InlineStyle::STRIKETHROUGH);
        }
        if self.underline > 0 {
            s = s.union(InlineStyle::UNDERLINE);
        }
        s
    }

    fn text(&mut self, text: &str, extra: InlineStyle) {
        // Spaces starting a line after a line break aren't text in Markdown.
        let after_break = self.runs.last().is_some_and(|r| r.text.ends_with('\n') && !r.style.contains(InlineStyle::RAW));
        let text = if after_break && !extra.contains(InlineStyle::CODE) { text.trim_start_matches(' ') } else { text };
        if text.is_empty() {
            return;
        }
        if self.open.is_none() {
            self.open = Some(Open::Implicit);
        }
        let style = self.style().union(extra);
        let link = self.links.last().map(|l| l.link.clone());
        // raw runs (images among them) stay apart
        if let Some(last) = self.runs.last_mut().filter(|l| !l.style.contains(InlineStyle::RAW)) {
            if last.style == style && last.link == link {
                last.text.push_str(text);
                return;
            }
        }
        self.runs.push(InlineRun { text: text.to_owned(), style, link });
    }

    fn raw(&mut self, text: &str) {
        if self.open.is_none() {
            self.open = Some(Open::Implicit);
        }
        self.runs.push(InlineRun { text: text.to_owned(), style: InlineStyle::RAW, link: None });
    }

    /// Ends the text block in progress, if any.
    fn close_text(&mut self) {
        let Some(open) = self.open.take() else { return };
        let runs = std::mem::take(&mut self.runs);
        let images = std::mem::take(&mut self.images);
        match open {
            Open::Heading(level, attributes) => self.push(Block::Heading { level, attributes }, runs),
            Open::Cell(cell) => self.push(Block::TableCell(cell), runs),
            Open::Paragraph | Open::Implicit => {
                // A paragraph of only images becomes image blocks; images among text stay
                // as written.
                let only_images = !images.is_empty()
                    && runs.iter().enumerate().all(|(i, r)| {
                        images.iter().any(|(at, _)| *at == i) || (r.link.is_none() && r.text.trim().is_empty())
                    });
                if only_images {
                    for (_, image) in images {
                        self.push(Block::Image(image), vec![]);
                    }
                } else if !runs.is_empty() || matches!(open, Open::Paragraph) {
                    self.push(Block::Paragraph, runs);
                }
            }
        }
    }

    fn slice(&self, range: Range<usize>) -> &str {
        self.source[range].trim_end_matches(['\n', '\r'])
    }

    fn open_container(&mut self, container: Container) {
        self.close_text();
        self.containers.push(container);
        self.container_starts.push(self.doc.paragraphs.len());
    }

    fn close_container(&mut self) {
        self.close_text();
        if self.container_starts.pop() == Some(self.doc.paragraphs.len()) {
            // An empty item or quote still needs a paragraph to exist in the editor.
            self.push(Block::Paragraph, vec![]);
        }
        self.containers.pop();
    }

    fn event(&mut self, event: Event, range: Range<usize>) {
        if self.skip > 0 {
            match event {
                Event::Start(Tag::FootnoteDefinition(_) | Tag::MetadataBlock(_)) => self.skip += 1,
                Event::End(TagEnd::FootnoteDefinition | TagEnd::MetadataBlock(_)) => self.skip -= 1,
                _ => {}
            }
            return;
        }
        if let Some((_, code)) = self.code.as_mut() {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (info, code) = self.code.take().unwrap();
                    let id = self.alternate(Slot::Code);
                    let code = code.strip_suffix('\n').unwrap_or(&code).to_owned();
                    for line in code.split('\n') {
                        let runs = if line.is_empty() { vec![] } else { vec![InlineRun::plain(line)] };
                        self.push(Block::Code { id, info: info.clone() }, runs);
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(html) = self.html.as_mut() {
            match event {
                Event::Html(t) | Event::Text(t) => html.push_str(&t),
                Event::End(TagEnd::HtmlBlock) => {
                    let html = self.html.take().unwrap();
                    self.raw_block(html.trim_end_matches(['\n', '\r']));
                }
                _ => {}
            }
            return;
        }
        if let Some((image, _)) = self.image.as_mut() {
            match event {
                Event::Text(t) | Event::Code(t) => image.alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (image, _) = self.image.take().unwrap();
                    // written inline, so it needs no reference definition
                    self.raw(&super::serialize::image(&image.alt, &image.source, &image.title));
                    self.images.push((self.runs.len() - 1, image));
                    if let Some(link) = self.links.last_mut() {
                        link.has_image = true;
                    }
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t, InlineStyle::NONE),
            Event::Code(t) => self.text(&t, InlineStyle::CODE),
            Event::SoftBreak => self.text(" ", InlineStyle::NONE),
            Event::HardBreak => self.text("\n", InlineStyle::NONE),
            Event::InlineHtml(h) | Event::Html(h) => self.inline_html(&h),
            Event::FootnoteReference(label) => self.raw(&format!("[^{label}]")),
            Event::Rule => {
                self.close_text();
                self.push(Block::ThematicBreak, vec![]);
            }
            Event::TaskListMarker(done) => {
                if let Some(Container::ListItem(item)) = self.containers.last_mut() {
                    item.task = Some(done);
                }
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => self.raw(&t),
        }
    }

    fn raw_block(&mut self, text: &str) {
        let id = self.alternate(Slot::Raw);
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            let runs = if line.is_empty() { vec![] } else { vec![InlineRun::plain(line)] };
            self.push(Block::Raw { id }, runs);
        }
    }

    /// `<u>` and `<ins>` underline the text up to their closing tag, and `<br>` is a line
    /// break. Any other HTML stays as written.
    fn inline_html(&mut self, html: &str) {
        let tag = html.trim().to_ascii_lowercase();
        match tag.as_str() {
            "<u>" | "<ins>" => self.underline += 1,
            "</u>" | "</ins>" => self.underline = self.underline.saturating_sub(1),
            "<br>" | "<br/>" | "<br />" => self.text("\n", InlineStyle::NONE),
            _ => self.raw(html),
        }
    }

    fn start(&mut self, tag: Tag, range: Range<usize>) {
        match tag {
            Tag::Paragraph => {
                self.close_text();
                self.open = Some(Open::Paragraph);
            }
            Tag::Heading { level, id, classes, attrs } => {
                self.close_text();
                let mut parts: Vec<String> = vec![];
                if let Some(id) = id {
                    parts.push(format!("#{id}"));
                }
                parts.extend(classes.iter().map(|c| format!(".{c}")));
                parts.extend(attrs.iter().map(|(k, v)| match v {
                    Some(v) => format!("{k}={v}"),
                    None => k.to_string(),
                }));
                let attributes = (!parts.is_empty()).then(|| format!("{{{}}}", parts.join(" ")));
                self.open = Some(Open::Heading(level as u8, attributes));
            }
            Tag::BlockQuote(kind) => {
                let id = self.alternate(Slot::Quote(self.containers.len()));
                let alert = kind.map(|k| match k {
                    BlockQuoteKind::Note => Alert::Note,
                    BlockQuoteKind::Tip => Alert::Tip,
                    BlockQuoteKind::Important => Alert::Important,
                    BlockQuoteKind::Warning => Alert::Warning,
                    BlockQuoteKind::Caution => Alert::Caution,
                });
                self.open_container(Container::Quote { id, alert });
            }
            Tag::CodeBlock(kind) => {
                self.close_text();
                let info = match kind {
                    CodeBlockKind::Fenced(info) => Some(info.to_string()).filter(|i| !i.is_empty()),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((info, String::new()));
            }
            Tag::HtmlBlock => {
                self.close_text();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.close_text();
                let id = self.alternate(Slot::List(self.containers.len()));
                self.lists.push((id, start.is_some(), start.unwrap_or(1), 0));
            }
            Tag::Item => {
                let Some(list) = self.lists.last_mut() else { return };
                list.3 += 1;
                let (list_id, ordered, start, n) = *list;
                let item_id = 1 + n % 2;
                self.open_container(Container::ListItem(ListItem { list_id, item_id, ordered, start, task: None }));
            }
            Tag::FootnoteDefinition(_) | Tag::MetadataBlock(_) => {
                self.close_text();
                let written = self.slice(range).to_owned();
                if matches!(tag, Tag::MetadataBlock(_)) && self.doc.paragraphs.is_empty() && self.doc.front_matter.is_none() {
                    self.doc.front_matter = Some(written);
                } else {
                    self.raw_block(&written);
                }
                self.skip = 1;
            }
            Tag::Table(alignments) => {
                self.close_text();
                let alignments: Rc<[Alignment]> = alignments
                    .iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Alignment::None,
                        pulldown_cmark::Alignment::Left => Alignment::Left,
                        pulldown_cmark::Alignment::Center => Alignment::Center,
                        pulldown_cmark::Alignment::Right => Alignment::Right,
                    })
                    .collect();
                let id = self.alternate(Slot::Table);
                self.table = Some(TableState { id, alignments, row: 0, column: 0 });
            }
            Tag::TableHead => {}
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.row += 1;
                    t.column = 0;
                }
            }
            Tag::TableCell => {
                if let Some(t) = &self.table {
                    let cell = TableCell { table_id: t.id, row: t.row, column: t.column, alignments: t.alignments.clone() };
                    self.open = Some(Open::Cell(cell));
                }
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, title, .. } => {
                if self.open.is_none() {
                    self.open = Some(Open::Implicit);
                }
                let link = Rc::new(Link { destination: dest_url.to_string(), title: title.to_string() });
                self.links.push(LinkState { link, first_run: self.runs.len(), has_image: false });
            }
            Tag::Image { dest_url, title, .. } => {
                let image = Image { source: dest_url.to_string(), alt: String::new(), title: title.to_string() };
                self.image = Some((image, range));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.close_text(),
            TagEnd::BlockQuote(_) | TagEnd::Item => self.close_container(),
            TagEnd::List(_) => {
                self.close_text();
                self.lists.pop();
            }
            TagEnd::TableCell => {
                self.close_text();
                if let Some(t) = self.table.as_mut() {
                    t.column += 1;
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                // every row has a cell for every column
                let Some(t) = &self.table else { return };
                let (id, row, alignments, from) = (t.id, t.row, t.alignments.clone(), t.column);
                for column in from..alignments.len() {
                    let cell = TableCell { table_id: id, row, column, alignments: alignments.clone() };
                    self.push(Block::TableCell(cell), vec![]);
                }
            }
            TagEnd::Table => self.table = None,
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                let Some(state) = self.links.pop() else { return };
                if state.has_image {
                    // a linked image (a badge…) is kept whole, written inline
                    let inner: String = self.runs[state.first_run..]
                        .iter()
                        .map(|r| if r.style.contains(InlineStyle::RAW) { r.text.clone() } else { super::serialize::escaped_label(&r.text) })
                        .collect();
                    let link = &state.link;
                    let written = format!(
                        "[{inner}]({}{})",
                        super::serialize::destination(&link.destination),
                        super::serialize::title(&link.title)
                    );
                    self.runs.truncate(state.first_run);
                    self.images.retain(|(i, _)| *i < state.first_run);
                    self.runs.push(InlineRun { text: written, style: InlineStyle::RAW, link: None });
                }
            }
            _ => {}
        }
    }
}
