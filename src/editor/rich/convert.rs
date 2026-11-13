//! Moves the document between Markdown and what the formatted editor shows.

use super::table::Table;
use super::{RichView, LITERAL_HASH, STYLE_TAGS};
use crate::document::{count_words, slugify, Heading};
use crate::markdown::model::*;
use crate::markdown::{parse, serialize};
use crate::widgets::{FitBox, FitImage};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

/// Hard line breaks are line separators in the text view, so they stay inside the paragraph.
pub const LINE_SEPARATOR: char = '\u{2028}';
pub const OBJECT: char = '\u{FFFC}';

impl RichView {
    /// Shows a Markdown text, replacing the document. Undo can't reach the previous one.
    pub fn render(&self, markdown: &str, base_dir: Option<gio::File>) {
        self.show_markdown(markdown, base_dir);
        self.forget_history();
    }

    pub(super) fn show_markdown(&self, markdown: &str, base_dir: Option<gio::File>) {
        let doc = parse::parse(markdown);
        self.show(&doc, base_dir);
    }

    pub fn show(&self, doc: &Document, base_dir: Option<gio::File>) {
        let imp = self.imp();
        imp.loading.set(true);
        imp.base_dir.replace(base_dir);
        imp.front_matter.replace(doc.front_matter.clone());
        imp.tables.borrow_mut().clear();
        imp.wide.borrow_mut().clear();
        imp.anchored.borrow_mut().clear();
        imp.typing_kind.replace(None);
        imp.pending.set(None);
        let paragraphs = &doc.paragraphs;
        // one line per paragraph, and one per table: its cells are an embedded grid
        let mut lines: Vec<(usize, usize, Kind)> = vec![];
        let mut i = 0;
        while i < paragraphs.len() {
            let p = &paragraphs[i];
            let mut j = i + 1;
            let mut kind = p.kind.clone();
            if let Some(cell) = p.kind.table_cell() {
                while j < paragraphs.len()
                    && paragraphs[j].kind.containers == p.kind.containers
                    && paragraphs[j].kind.table_cell().is_some_and(|c| c.table_id == cell.table_id)
                {
                    j += 1;
                }
                kind = kind.with_block(Block::TableCell(TableCell { row: 0, column: 0, ..cell.clone() }));
            }
            lines.push((i, j, kind));
            i = j;
        }
        let scale = self.scale();
        self.detached(|| {
            let buf = self.buf();
            buf.set_text("");
            let mut end = buf.end_iter();
            let mut previous_tags: Vec<gtk::TextTag> = vec![];
            // each line styled as it is added: styling it all afterwards took minutes
            for (n, (from, to, kind)) in lines.iter().enumerate() {
                let tags = self.line_tags(kind, n.checked_sub(1).map(|p| &lines[p].2), lines.get(n + 1).map(|l| &l.2), scale);
                if n > 0 {
                    self.insert_tagged(&mut end, "\n", &previous_tags);
                }
                let start = end.offset();
                let p = &paragraphs[*from];
                match &p.kind.block {
                    Block::TableCell(_) => self.insert_table_at(&mut end, &paragraphs[*from..*to]),
                    Block::Image(image) => self.insert_image(&mut end, kind, image),
                    Block::ThematicBreak => self.insert_rule_at(&mut end, kind),
                    _ => self.insert_runs(&mut end, kind, &p.runs, &tags),
                }
                let s = buf.iter_at_offset(start);
                for tag in &tags {
                    buf.apply_tag(tag, &s, &end);
                }
                previous_tags = tags;
            }
            // A table at the end gets a line after it, to go on writing below.
            if lines.last().is_some_and(|l| l.2.table_cell().is_some()) {
                self.insert_tagged(&mut end, "\n", &previous_tags);
            }
        });
        imp.loading.set(false);
        let buf = self.buf();
        buf.place_cursor(&buf.start_iter());
        self.update_typing_style();
        self.changed_layout();
        self.refit();
    }

    fn insert_tagged(&self, at: &mut gtk::TextIter, text: &str, tags: &[gtk::TextTag]) {
        if text.is_empty() {
            return;
        }
        let refs: Vec<&gtk::TextTag> = tags.iter().collect();
        self.buf().insert_with_tags(at, text, &refs);
    }

    fn insert_runs(&self, at: &mut gtk::TextIter, kind: &Kind, runs: &[InlineRun], line_tags: &[gtk::TextTag]) {
        let start = at.offset();
        let table = self.buf().tag_table();
        for run in runs {
            let shown = run.text.replace('\n', &LINE_SEPARATOR.to_string());
            let mut tags = line_tags.to_vec();
            for (flag, name) in STYLE_TAGS {
                if run.style.contains(flag) {
                    tags.extend(table.lookup(name));
                }
            }
            if let Some(link) = &run.link {
                tags.push(self.link_tag(link));
            }
            self.insert_tagged(at, &shown, &tags);
        }
        // A hash that survived parsing is literal: editing the paragraph must not make it a heading.
        let buf = self.buf();
        let first = buf.iter_at_offset(start);
        if kind.block == Block::Paragraph && first.char() == '#' {
            let mut second = first;
            second.forward_char();
            buf.apply_tag(&self.tag(LITERAL_HASH), &first, &second);
        }
    }

    fn anchor_widget(&self, at: &mut gtk::TextIter, kind: &Kind, widget: &impl IsA<gtk::Widget>) -> gtk::TextChildAnchor {
        let buf = self.buf();
        let start = at.offset();
        let anchor = buf.create_child_anchor(at);
        let s = buf.iter_at_offset(start);
        self.remove_kind_tags(&s, at);
        buf.apply_tag(&self.kind_tag(kind), &s, at);
        // while the buffer is away from the view, the widget joins it on return
        if self.buffer() == buf {
            self.add_child_at_anchor(widget, &anchor);
        }
        self.imp().anchored.borrow_mut().push((anchor.clone(), widget.clone().upcast()));
        self.imp().wide.borrow_mut().push(widget.clone().upcast());
        anchor
    }

    pub(super) fn insert_rule_at(&self, at: &mut gtk::TextIter, kind: &Kind) {
        let rule = gtk::Box::builder().css_classes(["md-rule"]).build();
        let fit = FitBox::new(&rule, true);
        fit.set_margin_top(8);
        fit.set_margin_bottom(8);
        self.anchor_widget(at, kind, &fit);
    }

    fn insert_image(&self, at: &mut gtk::TextIter, kind: &Kind, image: &Image) {
        let slot = gtk::Box::builder().valign(gtk::Align::End).build();
        let img = FitImage::new(None);
        if !image.alt.is_empty() {
            img.update_property(&[gtk::accessible::Property::Label(&image.alt)]);
        }
        slot.append(&img);
        self.anchor_widget(at, kind, &slot);
        let fallback_text = format!("🖼 {}", if image.alt.is_empty() { &image.source } else { &image.alt });
        let fallback = glib::clone!(
            #[weak]
            slot,
            #[weak]
            img,
            move || {
                if img.parent().is_some() {
                    slot.remove(&img);
                    slot.append(&gtk::Label::builder().label(&fallback_text).css_classes(["dim-label", "md-img-alt"]).build());
                }
            }
        );
        let Some(file) = self.resolve(&image.source) else {
            fallback();
            return;
        };
        let view = self.downgrade();
        glib::spawn_future_local(async move {
            match file.load_bytes_future().await {
                Ok((bytes, _)) => match crate::document::texture_from_bytes(&bytes) {
                    Some(p) => {
                        img.set_paintable(p);
                        if let Some(v) = view.upgrade() {
                            v.refit();
                        }
                    }
                    None => fallback(),
                },
                Err(_) => fallback(),
            }
        });
    }

    pub(super) fn insert_table_at(&self, at: &mut gtk::TextIter, cells: &[Paragraph]) {
        let Some(cell) = cells[0].kind.table_cell() else { return };
        let table = Table::new(self, cell.table_id, cell.alignments.clone(), cells);
        let kind = cells[0].kind.with_block(Block::TableCell(TableCell { row: 0, column: 0, ..cell.clone() }));
        let anchor = self.anchor_widget(at, &kind, &table.widget);
        self.imp().tables.borrow_mut().push((anchor, table));
    }

    pub(super) fn table_at(&self, anchor: &gtk::TextChildAnchor) -> Option<Rc<Table>> {
        self.imp().tables.borrow().iter().find(|(a, _)| a == anchor).map(|(_, t)| t.clone())
    }

    pub fn resolve(&self, src: &str) -> Option<gio::File> {
        if src.is_empty() {
            return None;
        }
        if let Some(scheme) = glib::Uri::peek_scheme(src) {
            return (scheme != "data").then(|| gio::File::for_uri(src));
        }
        let path = crate::document::percent_decode(src.split(['#', '?']).next().unwrap_or(""));
        if path.starts_with('/') {
            return Some(gio::File::for_path(path));
        }
        self.imp().base_dir.borrow().as_ref().map(|d| d.resolve_relative_path(path))
    }

    // ------------------------------------------------------------- reading

    /// The document as the editor holds it now.
    pub fn document(&self) -> Document {
        let buf = self.buf();
        let mut paragraphs: Vec<Paragraph> = vec![];
        for line in 0..buf.line_count() {
            let kind = self.kind_at_line(line);
            let (start, end) = self.line_bounds(line);
            if start == end && self.is_empty_last_line(line) && kind == Kind::body() {
                continue;
            }
            if let Some(anchor) = start.child_anchor() {
                if let Some(table) = self.table_at(&anchor) {
                    paragraphs.extend(table.paragraphs(&kind.containers));
                    continue;
                }
            }
            match kind.block {
                Block::Image(_) | Block::ThematicBreak => {
                    if start.child_anchor().is_some() {
                        paragraphs.push(Paragraph { kind, runs: vec![] });
                    } else {
                        // the attachment was deleted: what's left is text
                        let runs = self.runs(&start, &end, false);
                        paragraphs.push(Paragraph { kind: Kind::body().with_containers(kind.containers), runs });
                    }
                }
                Block::TableCell(_) => {
                    let runs = self.runs(&start, &end, false);
                    paragraphs.push(Paragraph { kind: Kind::body().with_containers(kind.containers), runs });
                }
                Block::Code { .. } | Block::Raw { .. } => {
                    let text: String = buf.text(&start, &end, false).replace(LINE_SEPARATOR, "\n").replace(OBJECT, "");
                    // a line break inside a code line is a line of code
                    for piece in text.split('\n') {
                        let runs = if piece.is_empty() { vec![] } else { vec![InlineRun::plain(piece)] };
                        paragraphs.push(Paragraph { kind: kind.clone(), runs });
                    }
                }
                _ => {
                    let runs = self.runs(&start, &end, kind.block.is_text());
                    paragraphs.push(Paragraph { kind, runs });
                }
            }
        }
        Document { front_matter: self.imp().front_matter.borrow().clone(), paragraphs }
    }

    pub fn markdown(&self) -> String {
        serialize::markdown(&self.document())
    }

    /// The inline runs between two iters: one run per change of style or link.
    pub fn runs(&self, start: &gtk::TextIter, end: &gtk::TextIter, styled: bool) -> Vec<InlineRun> {
        runs_between(start, end, styled, |it| self.style_at(it))
    }

    // ------------------------------------------------------------- headings

    /// The headings, for the Contents panel, with a mark at each.
    pub fn headings(&self) -> Vec<Heading> {
        if let Some(cached) = self.imp().headings.borrow().as_ref() {
            return cached.clone();
        }
        let buf = self.buf();
        let mut slugs: std::collections::HashMap<String, usize> = Default::default();
        let mut headings = vec![];
        for line in 0..buf.line_count() {
            let kind = self.kind_at_line(line);
            let Block::Heading { level, attributes } = &kind.block else { continue };
            let (start, end) = self.line_bounds(line);
            let title = buf.text(&start, &end, false).replace(LINE_SEPARATOR, " ").replace(OBJECT, "");
            let custom = attributes.as_deref().and_then(|a| {
                a.trim_matches(['{', '}']).split_whitespace().find_map(|p| p.strip_prefix('#')).map(str::to_owned)
            });
            let base = custom.unwrap_or_else(|| slugify(&title));
            let n = slugs.entry(base.clone()).or_insert(0);
            let slug = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
            *n += 1;
            let mark = buf.create_mark(None, &start, true);
            headings.push(Heading { level: *level, title: title.trim().to_owned(), slug, mark });
        }
        // the previous marks are no longer needed
        if let Some(old) = self.imp().headings.replace(Some(headings.clone())) {
            for h in old {
                if !h.mark.is_deleted() {
                    buf.delete_mark(&h.mark);
                }
            }
        }
        headings
    }

    pub fn find_heading(&self, slug: &str) -> Option<Heading> {
        let slug = crate::document::percent_decode(slug).to_lowercase();
        self.headings().into_iter().find(|h| h.slug == slug)
    }

    pub fn find_anchor(&self, slug: &str) -> Option<gtk::TextMark> {
        self.find_heading(slug).map(|h| h.mark)
    }

    pub fn heading_y(&self, heading: &Heading) -> i32 {
        let it = self.buf().iter_at_mark(&heading.mark);
        self.iter_location(&it).y()
    }

    /// Words of prose between two iters (the whole document by default), without code.
    pub fn count_words(&self, range: Option<(gtk::TextIter, gtk::TextIter)>) -> u64 {
        let buf = self.buf();
        let (start, end) = range.unwrap_or_else(|| buf.bounds());
        let mut total = 0;
        for line in start.line()..=end.line() {
            let kind = self.kind_at_line(line);
            if matches!(kind.block, Block::Code { .. } | Block::Raw { .. }) {
                continue;
            }
            let (mut s, mut e) = self.line_bounds(line);
            if s < start {
                s = start;
            }
            if e > end {
                e = end;
            }
            if s < e {
                total += count_words(&buf.text(&s, &e, false)) as u64;
            }
            if let Some(table) = s.child_anchor().and_then(|a| self.table_at(&a)) {
                total += table.count_words();
            }
        }
        total
    }
}

/// The inline runs of a buffer between two iters, for the editor and its table cells.
pub fn runs_between(
    start: &gtk::TextIter,
    end: &gtk::TextIter,
    styled: bool,
    style_at: impl Fn(&gtk::TextIter) -> InlineStyle,
) -> Vec<InlineRun> {
    let mut runs: Vec<InlineRun> = vec![];
    let mut it = *start;
    while it < *end {
        let mut next = it;
        if !next.forward_to_tag_toggle(None::<&gtk::TextTag>) || next > *end {
            next = *end;
        }
        let text: String = it.text(&next).replace(LINE_SEPARATOR, "\n").replace(OBJECT, "");
        if !text.is_empty() {
            let (style, link) = if styled {
                (style_at(&it), RichView::link_at_iter(&it).map(Rc::new))
            } else {
                (InlineStyle::NONE, None)
            };
            match runs.last_mut() {
                Some(last) if last.style == style && last.link == link => last.text.push_str(&text),
                _ => runs.push(InlineRun { text, style, link }),
            }
        }
        it = next;
    }
    runs
}
