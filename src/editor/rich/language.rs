//! The language of a code block. The fence that holds it isn't shown in the formatted editor,
//! so each block starts with a strip holding a chip with its language, which opens the menu
//! to choose it, as does right-click › Language.

use super::draw::{same_code, CODE_PAD};
use super::{RichView, PAGE_PAD};
use crate::i18n::tr;
use crate::markdown::code::{self, Language};
use crate::markdown::model::{Block, Kind};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, pango};

/// The value of `language.set` for a block without a language.
const NONE: &str = "";
/// Its value for a language the file names but the editor doesn't know.
const UNKNOWN: &str = "\u{1}unknown";

/// The strip above a code block that holds its chip, in pixels.
pub fn strip_height(scale: f64) -> i32 {
    chip_height(scale) + 6
}

fn chip_height(scale: f64) -> i32 {
    (12.0 * scale + 10.0).round() as i32
}

impl RichView {
    pub(super) fn setup_language(&self) {
        let group = gio::SimpleActionGroup::new();
        let set = gio::SimpleAction::new_stateful("set", Some(glib::VariantTy::STRING), &NONE.to_variant());
        set.connect_activate(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, param| {
                if let Some(name) = param.and_then(|p| p.str()) {
                    view.set_code_language((!name.is_empty()).then(|| name.to_owned()));
                }
            }
        ));
        group.add_action(&set);
        self.imp().language_action.replace(Some(set));
        let unknown = gio::SimpleAction::new_stateful("unknown", None, &true.to_variant());
        unknown.set_enabled(false);
        group.add_action(&unknown);
        self.insert_action_group("language", Some(&group));
        self.update_context_menu();

        // the chip: a hand over it, and a click opens the menu instead of placing the caret
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, x, y| {
                let over = view.chip_at(x, y).is_some();
                if over != view.imp().over_chip.replace(over) {
                    view.set_cursor_from_name(Some(if over { "pointer" } else { "text" }));
                }
            }
        ));
        self.add_controller(motion);
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |gesture, _, x, y| {
                if let Some((line, rect)) = view.chip_at(x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    view.show_language_menu(line, &rect);
                }
            }
        ));
        self.add_controller(click);
    }

    /// The info string of the code block at the caret: `None` outside code, `Some(None)` in
    /// code without a language.
    pub fn code_info_at_caret(&self) -> Option<Option<String>> {
        match self.kind_at_caret().block {
            Block::Code { info, .. } => Some(info),
            _ => None,
        }
    }

    /// Sets the language of the whole code block at the caret as one undo step; `None` leaves
    /// it without one. It is written after the fence.
    pub fn set_code_language(&self, info: Option<String>) {
        let line = self.caret_line();
        let kind = self.kind_at_line(line);
        let Block::Code { id, .. } = kind.block else { return };
        let block = Block::Code { id, info };
        let (first, last) = self.code_block_lines(line);
        self.edit(|| {
            for l in first..=last {
                let k = self.kind_at_line(l);
                self.set_line_kind(l, &k.with_block(block.clone()));
            }
        });
        self.update_context_menu();
    }

    /// The first and last lines of the code block a line is in.
    pub fn code_block_lines(&self, line: i32) -> (i32, i32) {
        let count = self.buf().line_count();
        let kind = self.kind_at_line(line);
        let (mut first, mut last) = (line, line);
        while first > 0 && same_code(&kind, Some(&self.kind_at_line(first - 1))) {
            first -= 1;
        }
        while last + 1 < count && same_code(&kind, Some(&self.kind_at_line(last + 1))) {
            last += 1;
        }
        (first, last)
    }

    // ------------------------------------------------------------- the chip

    fn chip_text(info: Option<&str>) -> String {
        format!("{} ⌄", code::label(info).unwrap_or_else(|| tr("Choose Language")))
    }

    /// Where a code block's chip is, in buffer coordinates, from its first line's top and look.
    pub(super) fn chip_rect(&self, kind: &Kind, line_y: i32, above: i32) -> graphene::Rect {
        let scale = self.scale();
        let info = match &kind.block {
            Block::Code { info, .. } => info.as_deref(),
            _ => None,
        };
        let layout = self.chip_layout(info);
        let (_, logical) = layout.pixel_extents();
        let h = chip_height(scale);
        let w = logical.width() + 16;
        let top = line_y + above - CODE_PAD - strip_height(scale) + 6;
        let right = self.width() - PAGE_PAD - 6;
        graphene::Rect::new((right - w) as f32, top as f32, w as f32, h as f32)
    }

    fn chip_layout(&self, info: Option<&str>) -> pango::Layout {
        let layout = self.create_pango_layout(Some(&Self::chip_text(info)));
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrFloat::new_scale(0.8));
        layout.set_attributes(Some(&attrs));
        layout
    }

    pub(super) fn draw_chip(&self, snapshot: &gtk::Snapshot, kind: &Kind, line: i32, rect: &graphene::Rect) {
        let fg = self.color();
        let info = match &kind.block {
            Block::Code { info, .. } => info.as_deref(),
            _ => None,
        };
        let background = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.1);
        let rounded = gsk::RoundedRect::from_rect(*rect, 6.0);
        snapshot.push_rounded_clip(&rounded);
        snapshot.append_color(&background, rect);
        snapshot.pop();
        let layout = self.chip_layout(info);
        let (_, logical) = layout.pixel_extents();
        let x = rect.x() + 8.0;
        let y = rect.y() + (rect.height() - logical.height() as f32) / 2.0;
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        snapshot.append_layout(&layout, &gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.75));
        snapshot.restore();
        self.imp().chips.borrow_mut().push((*rect, line));
    }

    /// The code block whose chip is at a point in the view: its first line, and the chip.
    pub fn chip_at(&self, x: f64, y: f64) -> Option<(i32, graphene::Rect)> {
        let (bx, by) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let point = graphene::Point::new(bx as f32, by as f32);
        let chips = self.imp().chips.borrow();
        chips.iter().find(|(r, _)| r.contains_point(&point)).map(|(r, line)| (*line, *r))
    }

    /// The menu of languages, under the block's chip. The caret moves into the block first
    /// unless it's already there, since the choice applies to the block at the caret.
    fn show_language_menu(&self, line: i32, chip: &graphene::Rect) {
        let (first, last) = self.code_block_lines(line);
        let caret = self.caret_line();
        if caret < first || caret > last {
            let buf = self.buf();
            buf.place_cursor(&self.line_bounds(first).0);
        }
        self.grab_focus();
        self.update_context_menu();
        let popover = gtk::PopoverMenu::from_model(Some(&self.language_menu()));
        popover.set_parent(self);
        let (wx, wy) = self.buffer_to_window_coords(gtk::TextWindowType::Widget, chip.x() as i32, chip.y() as i32);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(wx, wy, chip.width() as i32, chip.height() as i32)));
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_has_arrow(false);
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    /// "None", then the known languages by name. A language written in the file that isn't
    /// one of them comes first, marked and disabled, so it's clear the block has one.
    pub fn language_menu(&self) -> gio::Menu {
        let menu = gio::Menu::new();
        let current = self.code_info_at_caret().flatten();
        let top = gio::Menu::new();
        if current.as_deref().is_some_and(|i| Language::from_info(i).is_none() && code::label(Some(i)).is_some()) {
            top.append(code::label(current.as_deref()).as_deref(), Some("language.unknown"));
        }
        top.append(Some(&tr("None")), Some(&format!("language.set::{NONE}")));
        menu.append_section(None, &top);
        let known = gio::Menu::new();
        for language in Language::by_title() {
            known.append(Some(language.title()), Some(&format!("language.set::{}", language.fence_name())));
        }
        menu.append_section(None, &known);
        menu
    }

    /// The context menu: GTK's own items, then Format ▸, and Language ▸ in a code block. The
    /// language action's state checks the current one.
    pub(super) fn update_context_menu(&self) {
        let info = self.code_info_at_caret();
        let state = match &info {
            None | Some(None) => NONE.to_owned(),
            Some(Some(i)) => Language::from_info(i).map(|l| l.fence_name().to_owned()).unwrap_or_else(|| UNKNOWN.to_owned()),
        };
        let key = format!("{}|{state}|{info:?}", info.is_some());
        if self.imp().menu_key.replace(key.clone()) == key {
            return;
        }
        if let Some(action) = self.imp().language_action.borrow().as_ref() {
            action.set_state(&state.to_variant());
        }
        let menu = crate::editor::context_menu();
        if info.is_some() {
            menu.append_submenu(Some(&tr("Language")), &self.language_menu());
        }
        self.set_extra_menu(Some(&menu));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_grows_with_the_text() {
        assert!(strip_height(2.0) > strip_height(1.0));
        assert_eq!(RichView::chip_text(Some("ts")), "TypeScript ⌄");
        assert_eq!(RichView::chip_text(Some("rust")), "rust ⌄");
    }
}

impl RichView {
    /// The chips drawn last and their blocks' first lines, for tests.
    pub fn drawn_chips(&self) -> Vec<(graphene::Rect, i32)> {
        self.imp().chips.borrow().clone()
    }
}
