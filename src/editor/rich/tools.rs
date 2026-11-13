//! The formatting tools, applied directly to the formatted text. Each is one undo step and
//! works as a toggle, like its Markdown counterpart in the source editor.

use super::convert::OBJECT;
use super::{inline_style_at, RichView, STYLE_TAGS};
use crate::markdown::active::{ActiveFormats, ListKind};
use crate::markdown::format::{FormatCommand, TableDimensions};
use crate::markdown::model::*;
use adw::prelude::*;
use gtk::subclass::prelude::*;
use std::rc::Rc;

fn style_name(style: InlineStyle) -> &'static str {
    STYLE_TAGS.iter().find(|(s, _)| *s == style).map(|(_, n)| *n).unwrap_or("strong")
}

impl RichView {
    pub fn apply_format(&self, command: &FormatCommand) {
        if !self.is_editable() {
            return;
        }
        match command {
            FormatCommand::Bold => self.toggle_inline(InlineStyle::STRONG),
            FormatCommand::Italic => self.toggle_inline(InlineStyle::EMPHASIS),
            FormatCommand::Underline => self.toggle_inline(InlineStyle::UNDERLINE),
            FormatCommand::Strikethrough => self.toggle_inline(InlineStyle::STRIKETHROUGH),
            FormatCommand::InlineCode => self.toggle_inline(InlineStyle::CODE),
            FormatCommand::Link { text, destination } => self.insert_link(text, destination),
            _ if self.focused_cell().is_some() => {} // a cell holds only inline text
            FormatCommand::Heading(level) => self.set_heading(*level),
            FormatCommand::Quote => self.toggle_quote(),
            FormatCommand::BulletList => self.toggle_list(false),
            FormatCommand::NumberedList => self.toggle_list(true),
            FormatCommand::CodeBlock => self.toggle_code_block(),
            FormatCommand::Table(dimensions) => self.insert_table_block(dimensions),
            FormatCommand::HorizontalRule => self.insert_rule_block(),
        }
    }

    /// The text that a link would replace: the selection, in a cell or in the editor.
    pub fn selected_text(&self) -> String {
        let buf = self.focused_cell().map(|(_, c)| c.buffer()).unwrap_or_else(|| self.buf());
        buf.selection_bounds().map(|(s, e)| buf.text(&s, &e, false).replace(OBJECT, "")).unwrap_or_default()
    }

    // ------------------------------------------------------------- state

    /// The formatting at the caret or across the selection, for the pressed buttons and checks.
    pub fn active_formats(&self) -> ActiveFormats {
        let mut f = ActiveFormats::default();
        if let Some((table, cell)) = self.focused_cell() {
            let buf = cell.buffer();
            self.read_inline(&buf, table.typing_style(), &mut f);
            return f;
        }
        let kind = self.kind_at_caret();
        f.heading = kind.block.heading_level();
        f.code_block = kind.block.is_code();
        f.quote = kind.is_quoted();
        f.list = kind.containers.iter().rev().find_map(Container::list_item).map(|i| if i.ordered { ListKind::Numbered } else { ListKind::Bulleted });
        if !f.code_block {
            self.read_inline(&self.buf(), self.imp().typing_style.get(), &mut f);
        }
        f
    }

    fn read_inline(&self, buf: &gtk::TextBuffer, typing: InlineStyle, f: &mut ActiveFormats) {
        let (style, link) = match buf.selection_bounds() {
            None => {
                // what typing would produce, which is what bold with nothing selected acts on
                let caret = buf.iter_at_mark(&buf.get_insert());
                let mut before = caret;
                let link = !caret.starts_line() && before.backward_char() && RichView::link_at_iter(&before).is_some();
                (typing, link)
            }
            Some((s, e)) => {
                // on only when the whole selection has it, as the tools then remove it
                let mut common = InlineStyle(0xff);
                let mut linked = true;
                let mut it = s;
                while it < e {
                    if it.char() != OBJECT && it.char() != '\n' {
                        common = common.intersection(inline_style_at(&it));
                        linked &= RichView::link_at_iter(&it).is_some();
                    }
                    it.forward_char();
                }
                (if common == InlineStyle(0xff) { InlineStyle::NONE } else { common }, linked)
            }
        };
        f.strong = style.contains(InlineStyle::STRONG);
        f.emphasis = style.contains(InlineStyle::EMPHASIS);
        f.underline = style.contains(InlineStyle::UNDERLINE);
        f.strikethrough = style.contains(InlineStyle::STRIKETHROUGH);
        f.code = style.contains(InlineStyle::CODE);
        f.link = link;
    }

    // ------------------------------------------------------------- inline

    /// With nothing selected, the style applies to what is typed next.
    fn toggle_inline(&self, style: InlineStyle) {
        let cell = self.focused_cell();
        let buf = cell.as_ref().map(|(_, c)| c.buffer()).unwrap_or_else(|| self.buf());
        let Some((s, e)) = buf.selection_bounds() else {
            match &cell {
                Some((table, _)) => {
                    let current = table.typing_style();
                    table.set_typing_style(if current.contains(style) { current.without(style) } else { current.union(style) });
                }
                None => {
                    let current = self.imp().typing_style.get();
                    self.imp().typing_style.set(if current.contains(style) { current.without(style) } else { current.union(style) });
                }
            }
            self.queue_formats();
            return;
        };
        let mut applied = true;
        let mut it = s;
        while it < e {
            if it.char() != '\n' && it.char() != OBJECT && !inline_style_at(&it).contains(style) {
                applied = false;
                break;
            }
            it.forward_char();
        }
        let tag = buf.tag_table().lookup(style_name(style)).expect("style tag");
        self.edit(|| {
            if applied {
                buf.remove_tag(&tag, &s, &e);
            } else {
                buf.apply_tag(&tag, &s, &e);
            }
            if cell.is_none() {
                // code and raw lines don't take inline styles
                let (first, last) = (s.line(), e.line());
                self.restyle_lines(first, last);
            }
        });
    }

    fn insert_link(&self, text: &str, destination: &str) {
        let cell = self.focused_cell();
        let buf = cell.as_ref().map(|(_, c)| c.buffer()).unwrap_or_else(|| self.buf());
        let (s, e) = buf.selection_bounds().unwrap_or_else(|| {
            let c = buf.iter_at_mark(&buf.get_insert());
            (c, c)
        });
        let current = buf.text(&s, &e, false).replace(OBJECT, "");
        let typed = text.trim();
        let label = if !typed.is_empty() { typed.to_owned() } else if !current.is_empty() { current } else { destination.to_owned() };
        let style = if s < e { inline_style_at(&s) } else { cell.as_ref().map_or(self.imp().typing_style.get(), |(t, _)| t.typing_style()) };
        let link = self.link_tag(&Link { destination: destination.to_owned(), title: String::new() });
        let (start_offset, _) = (s.offset(), e.offset());
        self.edit(|| {
            let (mut s, mut e) = (buf.iter_at_offset(start_offset), buf.iter_at_offset(e.offset()));
            buf.delete(&mut s, &mut e);
            let mut at = buf.iter_at_offset(start_offset);
            let mut tags = vec![link.clone()];
            for (flag, name) in STYLE_TAGS {
                if style.contains(flag) {
                    tags.extend(buf.tag_table().lookup(name));
                }
            }
            let refs: Vec<&gtk::TextTag> = tags.iter().collect();
            buf.insert_with_tags(&mut at, &label, &refs);
            buf.place_cursor(&at);
        });
    }

    // ------------------------------------------------------------- paragraphs

    /// Changes the kinds of the selected lines, or of the empty last line, as one undo step.
    fn change_selected_kinds(&self, transform: impl FnOnce(Vec<Kind>) -> Vec<Kind>) {
        let (first, last) = self.selected_lines();
        let kinds: Vec<Kind> = (first..=last).map(|l| self.kind_at_line(l)).collect();
        let changed = transform(kinds);
        self.edit(|| {
            for (i, kind) in changed.iter().enumerate() {
                self.set_line_kind(first + i as i32, kind);
            }
        });
    }

    fn set_heading(&self, level: u8) {
        self.change_selected_kinds(|kinds| {
            let applied = level > 0 && kinds.iter().filter(|k| k.block.is_text()).all(|k| k.block.heading_level() == level);
            let block = if level == 0 || applied { Block::Paragraph } else { Block::Heading { level, attributes: None } };
            kinds.into_iter().map(|k| if k.block.is_text() { k.with_block(block.clone()) } else { k }).collect()
        });
    }

    fn toggle_quote(&self) {
        let id = self.make_id();
        self.change_selected_kinds(|kinds| {
            if kinds.iter().all(Kind::is_quoted) {
                return kinds
                    .into_iter()
                    .map(|k| {
                        let mut containers = k.containers.clone();
                        if let Some(i) = containers.iter().rposition(Container::is_quote) {
                            containers.remove(i);
                        }
                        k.with_containers(containers)
                    })
                    .collect();
            }
            let quote = Container::Quote { id, alert: None };
            kinds.into_iter().map(|k| if k.table_cell().is_none() { let mut c = k.containers.clone(); c.push(quote); k.with_containers(c) } else { k }).collect()
        });
    }

    fn toggle_list(&self, ordered: bool) {
        let (first, _) = self.selected_lines();
        let previous = (first > 0).then(|| self.kind_at_line(first - 1));
        let ids: Vec<u32> = (0..64).map(|_| self.make_id()).collect();
        let mut ids = ids.into_iter();
        let mut fresh = move || ids.next().unwrap_or(0);
        self.change_selected_kinds(|kinds| {
            let items: Vec<Option<ListItem>> = kinds.iter().map(|k| k.last_item().copied()).collect();
            if items.iter().all(|i| i.is_some_and(|i| i.ordered == ordered)) {
                return kinds.into_iter().map(|k| k.with_containers(k.containers[..k.containers.len() - 1].to_vec())).collect();
            }
            if items.iter().all(Option::is_some) {
                let list_id = fresh();
                return kinds
                    .into_iter()
                    .zip(items)
                    .map(|(k, item)| {
                        let item = item.unwrap();
                        let mut c = k.containers.clone();
                        *c.last_mut().unwrap() = Container::ListItem(ListItem { list_id, ordered, start: 1, ..item });
                        k.with_containers(c)
                    })
                    .collect();
            }
            // a new list right after one of the same kind continues it, as in a word processor
            let containers = kinds.first().map(|k| k.containers.clone()).unwrap_or_default();
            let (list_id, start) = match previous.as_ref().and_then(|p| {
                (p.containers.len() == containers.len() + 1 && p.containers[..containers.len()] == containers[..])
                    .then(|| p.last_item().copied())
                    .flatten()
                    .filter(|i| i.ordered == ordered)
            }) {
                Some(item) => (item.list_id, item.start),
                None => (fresh(), 1),
            };
            kinds
                .into_iter()
                .map(|k| {
                    if k.table_cell().is_some() {
                        return k;
                    }
                    let mut c = k.containers.clone();
                    c.push(Container::ListItem(ListItem { list_id, item_id: fresh(), ordered, start, task: None }));
                    k.with_containers(c)
                })
                .collect()
        });
    }

    fn toggle_code_block(&self) {
        let id = self.make_id();
        self.change_selected_kinds(|kinds| {
            if kinds.iter().all(|k| k.block.is_code()) {
                return kinds.into_iter().map(|k| k.with_block(Block::Paragraph)).collect();
            }
            let block = Block::Code { id, info: None };
            kinds.into_iter().map(|k| if k.block.is_text() || k.block.is_code() { k.with_block(block.clone()) } else { k }).collect()
        });
    }

    // ------------------------------------------------------------- insertions

    /// Inserts a block after the caret's paragraph, or in its place when it is empty, with an
    /// empty paragraph after it so typing can go on below.
    fn insert_block(&self, insert: impl FnOnce(&mut gtk::TextIter, &Kind)) -> i32 {
        let buf = self.buf();
        let line = self.caret_line();
        let current = self.kind_at_line(line);
        let containers = if current.table_cell().is_some() { vec![] } else { current.containers.clone() };
        let (start, end) = self.line_bounds(line);
        let empty = start == end && current.block.is_text();
        let mut block_line = 0;
        self.edit(|| {
            let mut at = if empty { start } else { end };
            if !empty {
                buf.insert(&mut at, "\n");
            }
            block_line = at.line();
            let kind = Kind { block: Block::Paragraph, containers: containers.clone() };
            insert(&mut at, &kind);
            buf.insert(&mut at, "\n");
            let after = at.line();
            self.set_line_kind(block_line, &self.kind_on_line_or(block_line, &kind));
            self.set_line_kind(after, &Kind::body().with_containers(containers.clone()));
            buf.place_cursor(&buf.iter_at_line(after).unwrap_or_else(|| buf.end_iter()));
        });
        block_line
    }

    fn kind_on_line_or(&self, line: i32, fallback: &Kind) -> Kind {
        let (s, _) = self.line_bounds(line);
        s.tags().iter().find_map(|t| self.kind_of_tag(t)).unwrap_or_else(|| fallback.clone())
    }

    fn retag(&self, start: i32, end: i32, kind: &Kind) {
        let buf = self.buf();
        let (s, e) = (buf.iter_at_offset(start), buf.iter_at_offset(end));
        self.remove_kind_tags(&s, &e);
        buf.apply_tag(&self.kind_tag(kind), &s, &e);
    }

    fn insert_rule_block(&self) {
        self.insert_block(|at, kind| {
            let kind = kind.with_block(Block::ThematicBreak);
            let start = at.offset();
            self.insert_rule_at(at, &kind);
            self.retag(start, at.offset(), &kind);
        });
    }

    fn insert_table_block(&self, dimensions: &TableDimensions) {
        let id = self.make_id();
        let alignments: Rc<[Alignment]> = vec![Alignment::None; dimensions.columns].into();
        let mut table = None;
        self.insert_block(|at, kind| {
            let mut cells = vec![];
            for row in 0..dimensions.rows {
                for column in 0..dimensions.columns {
                    let runs = if row == 0 { vec![InlineRun::plain(dimensions.headers[column].clone())] } else { vec![] };
                    let cell = TableCell { table_id: id, row, column, alignments: alignments.clone() };
                    cells.push(Paragraph::new(Block::TableCell(cell), kind.containers.clone(), runs));
                }
            }
            let start = at.offset();
            self.insert_table_at(at, &cells);
            let tkind = cells[0].kind.clone();
            self.retag(start, at.offset(), &tkind);
            table = self.imp().tables.borrow().last().map(|(_, t)| t.clone());
        });
        // the first header cell, selected, as the source editor selects its placeholder
        if let Some(table) = table {
            glib::idle_add_local_once(move || table.focus_first());
        }
    }
}

use gtk::glib;
