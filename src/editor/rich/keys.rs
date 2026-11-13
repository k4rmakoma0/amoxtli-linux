//! Keys of the formatted editor, as in Pages: Return continues a list or ends it on an empty
//! item, Delete at the start of a paragraph first undoes what the paragraph is, Tab nests list
//! items. Plus the heading shortcut: `# ` typed at the start of a paragraph makes it a heading.

use super::convert::LINE_SEPARATOR;
use super::{RichView, LITERAL_HASH};
use crate::markdown::model::*;
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::sync::LazyLock;

impl RichView {
    pub(super) fn setup_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if view.key(key, modifiers) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
            }
        ));
        self.add_controller(keys);
    }

    /// `true` when the key was handled here.
    pub fn key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        match key {
            gdk::Key::z | gdk::Key::Z if ctrl && shift => self.redo(),
            gdk::Key::z if ctrl => self.undo(),
            gdk::Key::y if ctrl => self.redo(),
            _ if !self.is_editable() || self.imp().preedit.get() || ctrl || alt => return false,
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                if shift && !self.kind_at_caret().block.is_code() {
                    self.edit(|| self.buf().insert_at_cursor(&LINE_SEPARATOR.to_string()));
                } else {
                    self.press_return();
                }
            }
            gdk::Key::Tab if !shift => return self.press_tab(false),
            gdk::Key::ISO_Left_Tab | gdk::Key::Tab => return self.press_tab(true),
            gdk::Key::BackSpace if !shift => return self.press_backspace(),
            gdk::Key::Down if !shift => return self.press_down(),
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------- Return

    /// Return continues a list, ends it on an empty item, and leaves a heading for body text.
    /// In a code block it always adds a line, blank ones included: code needs them.
    pub fn press_return(&self) {
        let buf = self.buf();
        let has_selection = buf.has_selection();
        let line = self.caret_line();
        let kind = self.kind_at_caret();
        let (start, end) = self.line_bounds(line);
        if !has_selection && start == end && !kind.containers.is_empty() && kind.block.is_text() {
            // Return on an empty item or quote line steps out of it.
            let outdented = kind.with_containers(self.outdented(&kind.containers));
            self.edit(|| self.set_line_kind(line, &outdented));
            return;
        }
        let at_end = buf.selection_bounds().map_or_else(|| buf.iter_at_mark(&buf.get_insert()), |(_, e)| e).offset() == end.offset();
        let next = self.kind_after_newline(&kind, at_end);
        self.edit(|| {
            buf.delete_selection(true, true);
            buf.insert_at_cursor("\n");
            let new_line = self.caret_line();
            self.set_line_kind(new_line, &next);
        });
        self.scroll_mark_onscreen(&buf.get_insert());
    }

    fn kind_after_newline(&self, kind: &Kind, at_end: bool) -> Kind {
        let mut next = kind.clone();
        match kind.block {
            Block::Heading { .. } if at_end => next.block = Block::Paragraph,
            Block::ThematicBreak | Block::Image(_) | Block::TableCell(_) => next.block = Block::Paragraph,
            _ => {}
        }
        if let Some(Container::ListItem(item)) = next.containers.last().copied() {
            *next.containers.last_mut().unwrap() = Container::ListItem(self.new_item(&item));
        }
        next
    }

    pub(super) fn new_item(&self, item: &ListItem) -> ListItem {
        ListItem { item_id: self.make_id(), task: item.task.map(|_| false), ..*item }
    }

    /// One level out. Leaving a nested list makes the paragraph a new item of the list around it.
    pub(super) fn outdented(&self, containers: &[Container]) -> Vec<Container> {
        let mut result = containers[..containers.len().saturating_sub(1)].to_vec();
        if let (Some(Container::ListItem(_)), Some(Container::ListItem(parent))) = (containers.last(), result.last().copied()) {
            *result.last_mut().unwrap() = Container::ListItem(ListItem { task: None, ..self.new_item(&parent) });
        }
        result
    }

    // ------------------------------------------------------------- Tab

    fn press_tab(&self, back: bool) -> bool {
        let kind = self.kind_at_caret();
        if kind.last_item().is_none() {
            return false;
        }
        if back {
            let line = self.caret_line();
            let outdented = kind.with_containers(self.outdented(&kind.containers));
            self.edit(|| self.set_line_kind(line, &outdented));
        } else {
            self.edit(|| self.indent_list_items());
        }
        true
    }

    /// Makes each selected item part of the item above it, in a list nested there.
    fn indent_list_items(&self) {
        let (first, last) = self.selected_lines();
        let mut previous = if first > 0 { self.kind_at_line(first - 1) } else { Kind::body() };
        for line in first..=last {
            let kind = self.kind_at_line(line);
            let depth = kind.containers.len().saturating_sub(1);
            let sibling = previous.containers.get(depth).and_then(Container::list_item).copied();
            let (Some(item), Some(sibling)) = (kind.last_item().copied(), sibling) else {
                previous = kind;
                continue;
            };
            if sibling.list_id != item.list_id {
                previous = kind;
                continue;
            }
            let nested_list = previous.containers.get(depth + 1).and_then(Container::list_item).map(|n| n.list_id).unwrap_or_else(|| self.make_id());
            let nested = ListItem { list_id: nested_list, item_id: self.make_id(), ordered: item.ordered, start: 1, task: item.task };
            let mut containers = previous.containers[..=depth].to_vec();
            containers.push(Container::ListItem(nested));
            let indented = kind.with_containers(containers);
            self.set_line_kind(line, &indented);
            previous = indented;
        }
    }

    /// The lines the selection covers; a selection ending at a line's start leaves that line out.
    pub(super) fn selected_lines(&self) -> (i32, i32) {
        let buf = self.buf();
        match buf.selection_bounds() {
            Some((s, e)) => {
                let last = if e.starts_line() && e.line() > s.line() { e.line() - 1 } else { e.line() };
                (s.line(), last)
            }
            None => {
                let line = self.caret_line();
                (line, line)
            }
        }
    }

    // ------------------------------------------------------------- Delete

    /// At the start of a paragraph, Delete first undoes what the paragraph is (a list item, a
    /// quote, a heading) before joining it with the one above.
    fn press_backspace(&self) -> bool {
        let buf = self.buf();
        if buf.has_selection() {
            return false;
        }
        let caret = buf.iter_at_mark(&buf.get_insert());
        let line = caret.line();
        if !caret.starts_line() || line == 0 {
            return false;
        }
        let kind = self.kind_at_line(line);
        let previous = self.kind_at_line(line - 1);
        match previous.block {
            Block::ThematicBreak | Block::Image(_) => {
                self.edit(|| {
                    let (mut start, _) = self.line_bounds(line - 1);
                    let mut end = self.line_end_with_break(line - 1);
                    buf.delete(&mut start, &mut end);
                });
                return true;
            }
            Block::TableCell(cell) if kind.table_cell().is_none() => {
                // into the table's last cell
                let table = self.imp().tables.borrow().iter().find(|(_, t)| t.id == cell.table_id).map(|(_, t)| t.clone());
                if let Some(cell) = table.and_then(|t| t.cells.last().and_then(|r| r.last()).cloned()) {
                    cell.grab_focus();
                    let b = cell.buffer();
                    b.place_cursor(&b.end_iter());
                }
                return true;
            }
            _ => {}
        }
        let changed = if !kind.containers.is_empty() && kind.containers != previous.containers {
            Some(kind.with_containers(self.outdented(&kind.containers)))
        } else if matches!(kind.block, Block::Heading { .. }) || (kind.block.is_code() && previous != kind) {
            Some(kind.with_block(Block::Paragraph))
        } else {
            None
        };
        match changed {
            Some(k) => {
                self.edit(|| self.set_line_kind(line, &k));
                true
            }
            None => false,
        }
    }

    // ------------------------------------------------------------- Down

    /// Down on the last line of a code block that ends the text leaves the block for a body
    /// line below it. Anywhere else it moves as usual.
    fn press_down(&self) -> bool {
        let buf = self.buf();
        if buf.has_selection() {
            return false;
        }
        let kind = self.kind_at_caret();
        let line = self.caret_line();
        if !kind.block.is_code() || line != buf.line_count() - 1 {
            return false;
        }
        let caret = buf.iter_at_mark(&buf.get_insert());
        let (_, end) = self.line_bounds(line);
        if self.iter_location(&caret).y() != self.iter_location(&end).y() {
            return false; // not on the block's last row yet
        }
        let body = kind.with_block(Block::Paragraph);
        self.edit(|| {
            if !self.is_empty_last_line(line) {
                let mut end = buf.end_iter();
                buf.insert(&mut end, "\n");
                buf.place_cursor(&buf.end_iter());
            }
            self.set_line_kind(buf.end_iter().line(), &body);
        });
        self.scroll_mark_onscreen(&buf.get_insert());
        true
    }

    // ------------------------------------------------------------- typing rules

    /// `# ` to `###### ` typed at the start of a body paragraph makes it a heading of that
    /// level, keeping its styles, list and quote. `\#` typed there is a literal `#`. Only for
    /// what is typed now: text pasted or of several lines stays as it is.
    pub(super) fn heading_input(&self, at: i32, text: &str) {
        static MARKER: LazyLock<fancy_regex::Regex> = LazyLock::new(|| fancy_regex::Regex::new(r"^#{1,6}[ \t]+(?=\S)").unwrap());
        if text.contains(['\n', LINE_SEPARATOR]) {
            return;
        }
        let buf = self.buf();
        let line = buf.iter_at_offset(at).line();
        let kind = self.kind_at_line(line);
        if kind.block != Block::Paragraph {
            return;
        }
        let (start, end) = self.line_bounds(line);
        let typed_end = (at + text.chars().count() as i32).min(end.offset());
        let prefix: String = start.text(&buf.iter_at_offset(typed_end)).to_string();
        let literal = self.tag(LITERAL_HASH);
        let plain = |len: i32| {
            let mut it = start;
            for _ in 0..len {
                if it.tags().iter().any(|t| t.name().as_deref() == Some("code")) || RichView::link_at_iter(&it).is_some() {
                    return false;
                }
                it.forward_char();
            }
            true
        };
        self.internally(|| {
            if at - start.offset() <= 1 && prefix.starts_with("\\#") && plain(2) {
                let (mut s, mut e) = (start, start);
                e.forward_char();
                buf.delete(&mut s, &mut e);
                let (s, _) = self.line_bounds(line);
                let mut e = s;
                e.forward_char();
                buf.apply_tag(&literal, &s, &e);
                return;
            }
            if !prefix.starts_with('#') || start.has_tag(&literal) {
                return;
            }
            let Ok(Some(m)) = MARKER.find(&prefix) else { return };
            let marker = m.as_str().chars().count() as i32;
            if !plain(marker) {
                return;
            }
            let level = prefix.chars().take_while(|c| *c == '#').count() as u8;
            let (mut s, _) = self.line_bounds(line);
            let mut e = buf.iter_at_offset(s.offset() + marker);
            buf.delete(&mut s, &mut e);
            self.set_line_kind(line, &kind.with_block(Block::Heading { level, attributes: None }));
        });
    }

    /// Runs changes that belong to the edit in progress: no undo step of their own.
    fn internally(&self, f: impl FnOnce()) {
        let imp = self.imp();
        imp.internal.set(imp.internal.get() + 1);
        f();
        imp.internal.set(imp.internal.get() - 1);
    }

    /// Deleting all of an element's text deletes the element too: a heading, a list item, a
    /// quote or a code line left empty becomes body text right away. Two empty lines keep
    /// what they are: a table cell, and a blank line between other lines of a code block.
    pub(super) fn clear_empty_element_at_caret(&self) {
        let buf = self.buf();
        if buf.has_selection() {
            return;
        }
        let line = self.caret_line();
        let (start, end) = self.line_bounds(line);
        if start != end {
            return;
        }
        let kind = self.kind_at_line(line);
        if kind == Kind::body() || kind.table_cell().is_some() {
            return;
        }
        if kind.block.is_code() {
            let count = buf.line_count();
            let before = line > 0 && self.kind_at_line(line - 1) == kind;
            let after = line + 1 < count && self.kind_at_line(line + 1) == kind;
            if before || after {
                return;
            }
        }
        self.internally(|| self.set_line_kind(line, &Kind::body()));
    }

    // ------------------------------------------------------------- pointer

    pub(super) fn setup_pointer(&self) {
        // Links open with Ctrl+click: a plain click puts the caret in the text to edit it.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        click.connect_released(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |gesture, _, x, y| {
                let ctrl = gesture.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
                if ctrl || !view.is_editable() {
                    if let Some(link) = view.link_at(x, y) {
                        view.activate_link(&link.destination);
                    }
                }
            }
        ));
        self.add_controller(click);
        self.set_has_tooltip(true);
        self.connect_query_tooltip(|view, x, y, keyboard, tooltip| {
            if keyboard {
                return false;
            }
            match view.link_at(x as f64, y as f64) {
                Some(link) => {
                    tooltip.set_text(Some(&format!("{}\n{}", link.destination, crate::i18n::tr("Ctrl+click to open"))));
                    true
                }
                None => false,
            }
        });
    }

    fn link_at(&self, x: f64, y: f64) -> Option<Link> {
        let (bx, by) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let it = self.iter_at_location(bx, by)?;
        RichView::link_at_iter(&it)
    }
}
