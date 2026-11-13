//! Tables in the formatted editor: a grid of small text views, one per cell, embedded on a
//! line of its own. GtkTextView can't lay out cells side by side, and a grid draws its whole
//! border every time. Cells are edited one by one; adding or removing rows and columns is
//! done in the source. They share the editor's tags, its undo history and its tools.

use super::convert::runs_between;
use super::{inline_style_at, RichView, STYLE_TAGS};
use crate::document::count_words;
use crate::markdown::model::*;
use crate::widgets::FitBox;
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Table {
    pub id: u32,
    pub alignments: Rc<[Alignment]>,
    pub widget: FitBox,
    /// Row by row; row 0 is the header.
    pub cells: Vec<Vec<gtk::TextView>>,
    view: glib::WeakRef<RichView>,
    typing_style: Cell<InlineStyle>,
    focused: RefCell<Option<(usize, usize)>>,
}

impl Table {
    pub fn new(view: &RichView, id: u32, alignments: Rc<[Alignment]>, paragraphs: &[Paragraph]) -> Rc<Self> {
        let columns = alignments.len().max(1);
        let rows = paragraphs.iter().filter_map(|p| p.kind.table_cell().map(|c| c.row)).max().unwrap_or(0) + 1;
        let grid = gtk::Grid::builder().css_classes(["md-table-frame"]).overflow(gtk::Overflow::Hidden).build();
        let tag_table = view.buf().tag_table();
        let mut cells = vec![];
        for row in 0..rows {
            let mut line = vec![];
            for column in 0..columns {
                let buffer = gtk::TextBuffer::new(Some(&tag_table));
                buffer.set_enable_undo(false);
                let justification = match alignments.get(column) {
                    Some(Alignment::Center) => gtk::Justification::Center,
                    Some(Alignment::Right) => gtk::Justification::Right,
                    _ => gtk::Justification::Left,
                };
                let cell = gtk::TextView::builder()
                    .buffer(&buffer)
                    .wrap_mode(gtk::WrapMode::WordChar)
                    .justification(justification)
                    .top_margin(7)
                    .bottom_margin(7)
                    .left_margin(12)
                    .right_margin(12)
                    .accepts_tab(false)
                    .hexpand(true)
                    .css_classes(["md-cell-text"])
                    .build();
                let frame = gtk::Box::builder().css_classes(["md-cell"]).build();
                if row == 0 {
                    frame.add_css_class("md-th");
                }
                if row == rows - 1 {
                    frame.add_css_class("md-last-row");
                }
                if column == columns - 1 {
                    frame.add_css_class("md-last-column");
                }
                frame.append(&cell);
                grid.attach(&frame, column as i32, row as i32, 1, 1);
                line.push(cell);
            }
            cells.push(line);
        }
        let table = Rc::new(Self {
            id,
            alignments,
            widget: FitBox::new(&grid, false),
            cells,
            view: view.downgrade(),
            typing_style: Cell::new(InlineStyle::NONE),
            focused: RefCell::new(None),
        });
        for p in paragraphs {
            let Some(cell) = p.kind.table_cell() else { continue };
            if let Some(text_view) = table.cells.get(cell.row).and_then(|r| r.get(cell.column)) {
                table.fill(text_view, &p.runs);
            }
        }
        table.connect();
        table.size_columns();
        table
    }

    /// A wrapping text view asks for almost no width, so each column asks for its widest
    /// text, up to a limit past which cells wrap.
    fn size_columns(&self) {
        let Some(columns) = self.cells.first().map(Vec::len) else { return };
        for column in 0..columns {
            let widest = self
                .cells
                .iter()
                .filter_map(|row| row.get(column))
                .map(|cell| {
                    let buf = cell.buffer();
                    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                    let layout = cell.create_pango_layout(Some(&text));
                    layout.pixel_size().0
                })
                .max()
                .unwrap_or(0);
            let width = (widest + 28).clamp(56, 280);
            for row in &self.cells {
                if let Some(cell) = row.get(column) {
                    cell.set_size_request(width, -1);
                }
            }
        }
    }

    fn fill(&self, cell: &gtk::TextView, runs: &[InlineRun]) {
        let Some(view) = self.view.upgrade() else { return };
        let buf = cell.buffer();
        let mut end = buf.end_iter();
        for run in runs {
            let mut tags = vec![];
            for (flag, name) in STYLE_TAGS {
                if run.style.contains(flag) {
                    tags.extend(buf.tag_table().lookup(name));
                }
            }
            if let Some(link) = &run.link {
                tags.push(view.link_tag(link));
            }
            let refs: Vec<&gtk::TextTag> = tags.iter().collect();
            buf.insert_with_tags(&mut end, &run.text.replace('\n', " "), &refs);
        }
    }

    fn connect(self: &Rc<Self>) {
        for (r, row) in self.cells.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let buf = cell.buffer();
                let weak = Rc::downgrade(self);
                let key = (self.id as i32).wrapping_mul(4096).wrapping_add((r * 64 + c) as i32);
                buf.connect_insert_text(move |_, _, text| {
                    if let Some(view) = weak.upgrade().and_then(|t| t.view.upgrade()) {
                        view.cell_before_change(key, text.chars().count() == 1 && !text.contains('\n'));
                    }
                });
                let weak = Rc::downgrade(self);
                buf.connect_delete_range(move |_, s, e| {
                    if let Some(view) = weak.upgrade().and_then(|t| t.view.upgrade()) {
                        view.cell_before_change(key, (e.offset() - s.offset()).abs() <= 1);
                    }
                });
                let weak = Rc::downgrade(self);
                buf.connect_local("insert-text", true, move |values| {
                    let table = weak.upgrade()?;
                    let at = values[1].get::<gtk::TextIter>().ok()?;
                    let text = values[2].get::<String>().ok()?;
                    let buf = values[0].get::<gtk::TextBuffer>().ok()?;
                    let start = buf.iter_at_offset(at.offset() - text.chars().count() as i32);
                    // typed text takes the style typing has, as in the editor
                    let style = table.typing_style.get();
                    for (flag, name) in STYLE_TAGS {
                        if let Some(tag) = buf.tag_table().lookup(name) {
                            if style.contains(flag) {
                                buf.apply_tag(&tag, &start, &at);
                            }
                        }
                    }
                    None
                });
                let weak = Rc::downgrade(self);
                buf.connect_end_user_action(move |_| {
                    let Some(table) = weak.upgrade() else { return };
                    table.size_columns();
                    if let Some(view) = table.view.upgrade() {
                        view.cell_changed();
                    }
                });
                let weak = Rc::downgrade(self);
                buf.connect_mark_set(move |buf, _, mark| {
                    let Some(table) = weak.upgrade() else { return };
                    if mark == &buf.get_insert() {
                        let caret = buf.iter_at_mark(mark);
                        let mut before = caret;
                        let style = if caret.starts_line() || !before.backward_char() { InlineStyle::NONE } else { inline_style_at(&before) };
                        table.typing_style.set(style);
                    }
                    if let Some(view) = table.view.upgrade() {
                        view.cell_selection_changed();
                    }
                });
                let weak = Rc::downgrade(self);
                cell.connect_has_focus_notify(move |cell| {
                    let Some(table) = weak.upgrade() else { return };
                    if cell.has_focus() {
                        table.focused.replace(Some((r, c)));
                        if let Some(view) = table.view.upgrade() {
                            view.cell_selection_changed();
                        }
                    }
                });
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = Rc::downgrade(self);
                keys.connect_key_pressed(move |_, key, _, modifiers| {
                    let Some(table) = weak.upgrade() else { return glib::Propagation::Proceed };
                    table.key(r, c, key, modifiers)
                });
                cell.add_controller(keys);
            }
        }
    }

    fn key(&self, r: usize, c: usize, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let Some(view) = self.view.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Tab if !ctrl => {
                self.move_to(r, c, 1);
            }
            gdk::Key::ISO_Left_Tab => {
                self.move_to(r, c, -1);
            }
            gdk::Key::Return | gdk::Key::KP_Enter if !ctrl => {
                if !shift && !self.move_to(r, c, 1) {
                    view.leave_table(self.id);
                }
            }
            gdk::Key::z | gdk::Key::Z if ctrl && shift => view.redo(),
            gdk::Key::z if ctrl => view.undo(),
            gdk::Key::y if ctrl => view.redo(),
            _ => return glib::Propagation::Proceed,
        };
        glib::Propagation::Stop
    }

    /// Moves to the next or previous cell, selecting its text. `false` past either end.
    fn move_to(&self, r: usize, c: usize, step: isize) -> bool {
        let columns = self.cells[0].len() as isize;
        let index = r as isize * columns + c as isize + step;
        if index < 0 || index >= columns * self.cells.len() as isize {
            return index >= 0; // Tab past the last cell stays put; Return leaves
        }
        let cell = &self.cells[(index / columns) as usize][(index % columns) as usize];
        cell.grab_focus();
        let buf = cell.buffer();
        buf.select_range(&buf.start_iter(), &buf.end_iter());
        true
    }

    pub fn focus_first(&self) {
        if let Some(cell) = self.cells.first().and_then(|r| r.first()) {
            cell.grab_focus();
            let buf = cell.buffer();
            buf.select_range(&buf.start_iter(), &buf.end_iter());
        }
    }

    pub fn focused_cell(&self) -> Option<gtk::TextView> {
        self.cells.iter().flatten().find(|c| c.has_focus()).cloned()
    }

    pub fn typing_style(&self) -> InlineStyle {
        self.typing_style.get()
    }

    pub fn set_typing_style(&self, style: InlineStyle) {
        self.typing_style.set(style);
    }

    pub fn paragraphs(&self, containers: &[Container]) -> Vec<Paragraph> {
        let mut out = vec![];
        for (row, line) in self.cells.iter().enumerate() {
            for (column, cell) in line.iter().enumerate() {
                let buf = cell.buffer();
                let runs = runs_between(&buf.start_iter(), &buf.end_iter(), true, inline_style_at);
                let position = TableCell { table_id: self.id, row, column, alignments: self.alignments.clone() };
                out.push(Paragraph::new(Block::TableCell(position), containers.to_vec(), runs));
            }
        }
        out
    }

    pub fn count_words(&self) -> u64 {
        self.cells.iter().flatten().map(|c| {
            let buf = c.buffer();
            count_words(&buf.text(&buf.start_iter(), &buf.end_iter(), false)) as u64
        }).sum()
    }

    pub fn restyle(&self) {
        for cell in self.cells.iter().flatten() {
            cell.queue_draw();
        }
    }
}

impl RichView {
    /// A cell is about to change: an undo step opens unless typing in it continues.
    pub(super) fn cell_before_change(&self, cell: i32, typing: bool) {
        let imp = self.imp();
        if imp.loading.get() {
            return;
        }
        let continues = typing
            && imp.group.get() == super::Group::Cell
            && imp.group_caret.get() == cell
            && imp.group_time.get().is_some_and(|t| t.elapsed().as_secs_f64() < 2.0);
        if !continues {
            self.checkpoint();
        }
        imp.group.set(if typing { super::Group::Cell } else { super::Group::None });
        imp.group_caret.set(cell);
        imp.group_time.set(Some(std::time::Instant::now()));
    }

    pub(super) fn cell_changed(&self) {
        if !self.imp().loading.get() {
            self.notify_edited();
            self.queue_formats();
        }
    }

    pub(super) fn cell_selection_changed(&self) {
        self.queue_formats();
    }

    /// Every table, for tests.
    pub fn imp_tables(&self) -> Vec<(gtk::TextChildAnchor, Rc<Table>)> {
        self.imp().tables.borrow().clone()
    }

    /// The table cell with keyboard focus, and its table.
    pub fn focused_cell(&self) -> Option<(Rc<Table>, gtk::TextView)> {
        self.imp().tables.borrow().iter().find_map(|(_, t)| t.focused_cell().map(|c| (t.clone(), c)))
    }

    /// Return in the last cell goes to the line after the table.
    pub(super) fn leave_table(&self, id: u32) {
        let anchor = self.imp().tables.borrow().iter().find(|(_, t)| t.id == id).map(|(a, _)| a.clone());
        let Some(anchor) = anchor else { return };
        let buf = self.buf();
        let it = buf.iter_at_child_anchor(&anchor);
        let mut next = it;
        if next.forward_line() {
            buf.place_cursor(&next);
        } else {
            // the table ends the text: a line to go on writing
            let end = buf.end_iter();
            let kind = self.kind_at_line(end.line());
            self.edit(|| {
                let mut end = buf.end_iter();
                buf.insert(&mut end, "\n");
            });
            self.set_line_kind(buf.end_iter().line(), &Kind::body().with_containers(kind.containers));
            buf.place_cursor(&buf.end_iter());
        }
        self.grab_focus();
    }
}
