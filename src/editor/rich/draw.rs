//! What the formatted editor draws under the text: quote bars, code backgrounds and list
//! markers, which aren't text, so typing never breaks them and numbers follow the items. Also
//! the spacing of each line, which depends on its neighbors.

use super::{RichView, PAGE_PAD};
use crate::markdown::model::{Alert, Block, Container, Kind};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk, pango};
use std::collections::HashMap;

/// Space between a code block's background and its text.
pub(super) const CODE_PAD: i32 = 8;
const BULLETS: [&str; 3] = ["•", "◦", "▪"];

/// The settings of a line's look tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    pub left: i32,
    pub right: i32,
    pub above: i32,
    pub below: i32,
    pub code: bool,
}

fn indent_px(containers: &[Container], scale: f64) -> i32 {
    containers
        .iter()
        .map(|c| match c {
            Container::Quote { .. } => (20.0 * scale).round() as i32,
            Container::ListItem(_) => (26.0 * scale).round() as i32,
        })
        .sum()
}

pub(super) fn same_code(a: &Kind, b: Option<&Kind>) -> bool {
    match (&a.block, b.map(|k| &k.block)) {
        (Block::Code { id: x, .. }, Some(Block::Code { id: y, .. })) => x == y && Some(&a.containers) == b.map(|k| &k.containers),
        (Block::Raw { id: x }, Some(Block::Raw { id: y })) => x == y,
        _ => false,
    }
}

fn same_list(a: &Kind, b: Option<&Kind>) -> bool {
    match (a.containers.first(), b.and_then(|k| k.containers.first())) {
        (Some(Container::ListItem(x)), Some(Container::ListItem(y))) => x.list_id == y.list_id,
        _ => false,
    }
}

fn starts_alert(kind: &Kind, previous: Option<&Kind>) -> Option<(usize, Alert)> {
    kind.containers.iter().enumerate().find_map(|(depth, c)| match c {
        Container::Quote { alert: Some(alert), .. } if previous.and_then(|p| p.containers.get(depth)) != Some(c) => Some((depth, *alert)),
        _ => None,
    })
}

impl Look {
    /// Spacing depends on the neighbors: lines of one code block and items of one list sit
    /// close together, and leaving either leaves room.
    pub fn of(kind: &Kind, previous: Option<&Kind>, next: Option<&Kind>, scale: f64) -> Look {
        let unit = 15.0 * scale;
        let px = |f: f64| (unit * f).round() as i32;
        let indent = indent_px(&kind.containers, scale);
        let is_code = kind.block.is_code();
        let pad = if is_code { 12 } else { 0 };
        let first_of_block = !same_code(kind, previous);
        let last_of_block = !same_code(kind, next);
        let mut above = match &kind.block {
            Block::Heading { level, .. } => {
                if *level <= 2 { px(1.0) } else { px(0.5) }
            }
            // and room for the strip with the language chip
            Block::Code { .. } if first_of_block => px(0.4) + CODE_PAD + super::language::strip_height(scale),
            _ => 0,
        };
        let below = match &kind.block {
            Block::Code { .. } if last_of_block => px(0.9) + CODE_PAD,
            Block::Code { .. } | Block::Raw { .. } if !last_of_block => 0,
            _ if same_list(kind, next) => px(0.3),
            _ => px(0.75),
        };
        if starts_alert(kind, previous).is_some() {
            above += px(1.7); // room for the alert's title
        }
        Look { left: PAGE_PAD + indent + pad, right: PAGE_PAD + pad, above, below, code: is_code }
    }
}

/// Surfaces drawn here; the table's header and lines are in the style sheet.
#[derive(Clone, Copy)]
pub enum Surface {
    Code,
}

/// The system's fills barely show in dark mode: black or white with an opacity instead.
pub fn surface(surface: Surface, dark: bool, high_contrast: bool) -> gdk::RGBA {
    let alpha = match (surface, high_contrast, dark) {
        (Surface::Code, true, _) => 0.16,
        (Surface::Code, false, true) => 0.13,
        (Surface::Code, false, false) => 0.075,
    };
    let v = if dark { 1.0 } else { 0.0 };
    gdk::RGBA::new(v, v, v, alpha)
}

pub fn alert_kind(alert: Alert) -> pulldown_cmark::BlockQuoteKind {
    match alert {
        Alert::Note => pulldown_cmark::BlockQuoteKind::Note,
        Alert::Tip => pulldown_cmark::BlockQuoteKind::Tip,
        Alert::Important => pulldown_cmark::BlockQuoteKind::Important,
        Alert::Warning => pulldown_cmark::BlockQuoteKind::Warning,
        Alert::Caution => pulldown_cmark::BlockQuoteKind::Caution,
    }
}

/// A list item's marker on the line that starts it.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    /// Index of the item's container in the line's containers.
    pub depth: usize,
    /// How many lists the item is nested in.
    pub level: usize,
    /// The item's position in its list, from 1.
    pub index: u64,
    pub text: String,
}

pub type LineMarkers = Vec<Marker>;

/// Numbers items in document order: each new item takes the next number of its list, so
/// items renumber as they come and go.
#[derive(Default)]
pub struct Numbering {
    counts: HashMap<u32, u64>,
    previous: Vec<Container>,
}

impl Numbering {
    pub fn markers(&mut self, containers: &[Container]) -> LineMarkers {
        let mut markers = vec![];
        for (depth, container) in containers.iter().enumerate() {
            let Container::ListItem(item) = container else { continue };
            if self.previous.get(depth) == Some(container) {
                continue;
            }
            // ids repeat apart from their neighbors: a list that doesn't continue the one above starts at 1
            let continues = self.previous.get(depth).and_then(Container::list_item).is_some_and(|p| p.list_id == item.list_id);
            if !continues {
                self.counts.remove(&item.list_id);
            }
            let index = self.counts.get(&item.list_id).copied().unwrap_or(0) + 1;
            self.counts.insert(item.list_id, index);
            let level = containers[..depth].iter().filter(|c| c.list_item().is_some()).count();
            let text = match (item.ordered, item.task) {
                (true, _) => format!("{}.", item.start + index - 1),
                (false, Some(done)) => if done { "☑" } else { "☐" }.to_owned(),
                (false, None) => BULLETS[level % BULLETS.len()].to_owned(),
            };
            markers.push(Marker { depth, level, index, text });
        }
        self.previous = containers.to_vec();
        markers
    }
}

impl RichView {
    /// The markers of every line, and of the empty last line as its typing kind has it.
    pub fn line_markers(&self) -> Vec<LineMarkers> {
        if let Some(cached) = self.imp().markers.borrow().as_ref() {
            return cached.clone();
        }
        let buf = self.buf();
        let mut numbering = Numbering::default();
        let mut all = vec![];
        for line in 0..buf.line_count() {
            let kind = if self.is_empty_last_line(line) {
                self.empty_last_kind().unwrap_or_else(Kind::body)
            } else {
                self.kind_at_line(line)
            };
            all.push(numbering.markers(&kind.containers));
        }
        self.imp().markers.replace(Some(all.clone()));
        all
    }

    fn line_kind_for_drawing(&self, line: i32) -> Option<Kind> {
        if self.is_empty_last_line(line) {
            return self.empty_last_kind();
        }
        Some(self.kind_at_line(line))
    }

    pub(super) fn paint(&self, snapshot: &gtk::Snapshot) {
        let buf = self.buf();
        let visible = self.visible_rect();
        let sm = adw::StyleManager::default();
        let (dark, hc) = (sm.is_dark(), sm.is_high_contrast());
        let code_bg = surface(Surface::Code, dark, hc);
        let fg = self.color();
        let bar = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.3);
        let marker_color = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.75);
        let width = self.width();
        let scale = self.scale();
        let markers = self.line_markers();
        let count = buf.line_count();

        self.imp().chips.borrow_mut().clear();
        let (start, _) = self.line_at_y(visible.y());
        let mut line = start.line();
        let mut previous = (line > 0).then(|| self.line_kind_for_drawing(line - 1)).flatten();
        let mut current = self.line_kind_for_drawing(line);
        let mut code_drawn_to = -1;
        while line < count {
            let it = buf.iter_at_line(line).unwrap_or_else(|| buf.end_iter());
            let (y, h) = self.line_yrange(&it);
            if y > visible.y() + visible.height() {
                break;
            }
            let next = (line + 1 < count).then(|| self.line_kind_for_drawing(line + 1)).flatten();
            if let Some(kind) = &current {
                let look = Look::of(kind, previous.as_ref(), next.as_ref(), scale);
                // a code block's background in one piece, rounded at its ends
                if kind.block.is_code() && line > code_drawn_to {
                    let first = !same_code(kind, previous.as_ref());
                    let top = if first { y + look.above - CODE_PAD - super::language::strip_height(scale) } else { y };
                    let mut last_line = line;
                    let mut last_kind = kind.clone();
                    let mut after = next.clone();
                    while same_code(&last_kind, after.as_ref()) && last_line + 1 < count {
                        last_line += 1;
                        last_kind = after.clone().unwrap();
                        after = (last_line + 1 < count).then(|| self.line_kind_for_drawing(last_line + 1)).flatten();
                    }
                    let last_iter = buf.iter_at_line(last_line).unwrap_or_else(|| buf.end_iter());
                    let (ly, lh) = self.line_yrange(&last_iter);
                    let before_last = if last_line == line { previous.as_ref() } else { Some(&last_kind) };
                    let last_look = Look::of(&last_kind, before_last, after.as_ref(), scale);
                    let bottom = ly + lh - last_look.below + CODE_PAD;
                    let x = PAGE_PAD + indent_px(&kind.containers, scale);
                    let rect = graphene::Rect::new(x as f32, top as f32, (width - PAGE_PAD - x) as f32, (bottom - top) as f32);
                    let r = |on: bool| graphene::Size::new(if on { 8.0 } else { 0.0 }, if on { 8.0 } else { 0.0 });
                    let rounded = gsk::RoundedRect::new(rect, r(first), r(first), r(true), r(true));
                    snapshot.push_rounded_clip(&rounded);
                    snapshot.append_color(&code_bg, &rect);
                    snapshot.pop();
                    if first {
                        let chip = self.chip_rect(kind, y, look.above);
                        self.draw_chip(snapshot, kind, line, &chip);
                    }
                    code_drawn_to = last_line;
                }
                // quote bars, one per quote the line is in
                for (depth, c) in kind.containers.iter().enumerate() {
                    let Container::Quote { alert, .. } = c else { continue };
                    let x = PAGE_PAD + indent_px(&kind.containers[..depth], scale) + 2;
                    let color = alert.map(|a| crate::document::alert_color(alert_kind(a), dark)).unwrap_or(bar);
                    let rect = graphene::Rect::new(x as f32, y as f32, 3.0, h as f32);
                    snapshot.append_color(&color, &rect);
                }
                if let Some((depth, alert)) = starts_alert(kind, previous.as_ref()) {
                    let x = PAGE_PAD + indent_px(&kind.containers[..=depth], scale);
                    let layout = self.create_pango_layout(Some(&crate::document::alert_title(alert_kind(alert))));
                    let attrs = pango::AttrList::new();
                    attrs.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
                    layout.set_attributes(Some(&attrs));
                    let color = crate::document::alert_color(alert_kind(alert), dark);
                    let ty = y + look.above - (15.0 * scale * 1.5).round() as i32;
                    self.draw_layout(snapshot, &layout, x as f32, ty as f32, &color);
                }
                // list markers, right-aligned before the item's text
                if let Some(line_markers) = markers.get(line as usize) {
                    for m in line_markers {
                        let right = PAGE_PAD + indent_px(&kind.containers[..=m.depth], scale) - 7;
                        let layout = self.create_pango_layout(Some(&m.text));
                        if let Block::Heading { level, .. } = kind.block {
                            let factor = [2.0, 1.69, 1.31, 1.15, 1.0, 0.85][(level.clamp(1, 6) - 1) as usize];
                            let attrs = pango::AttrList::new();
                            attrs.insert(pango::AttrFloat::new_scale(factor));
                            layout.set_attributes(Some(&attrs));
                        }
                        let (_, logical) = layout.pixel_extents();
                        self.draw_layout(snapshot, &layout, (right - logical.width()) as f32, (y + look.above) as f32, &marker_color);
                    }
                }
            }
            line += 1;
            previous = current;
            current = next;
        }
    }

    fn draw_layout(&self, snapshot: &gtk::Snapshot, layout: &pango::Layout, x: f32, y: f32, color: &gdk::RGBA) {
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        snapshot.append_layout(layout, color);
        snapshot.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::model::ListItem;

    fn item(list: u32, item: u32, ordered: bool, start: u64) -> Container {
        Container::ListItem(ListItem { list_id: list, item_id: item, ordered, start, task: None })
    }

    #[test]
    fn numbering_follows_the_items() {
        let mut n = Numbering::default();
        let texts: Vec<String> = [
            vec![item(1, 2, true, 3)],
            vec![item(1, 2, true, 3)], // second paragraph of the same item: no marker
            vec![item(1, 4, true, 3)],
            vec![item(1, 4, true, 3), item(5, 6, false, 1)],
        ]
        .iter()
        .flat_map(|c| n.markers(c))
        .map(|m| m.text)
        .collect();
        assert_eq!(texts, ["3.", "4.", "◦"]);
        // a list further down with the same id counts from its start again
        let again: Vec<String> = [vec![], vec![item(1, 2, true, 1)]].iter().flat_map(|c| n.markers(c)).map(|m| m.text).collect();
        assert_eq!(again, ["1."]);
    }

    #[test]
    fn spacing() {
        let code = |id| Kind { block: Block::Code { id, info: None }, containers: vec![] };
        let look = Look::of(&code(1), None, Some(&code(1)), 1.0);
        assert_eq!(look.below, 0);
        assert!(look.above > 0);
        let inner = Look::of(&code(1), Some(&code(1)), Some(&code(1)), 1.0);
        assert_eq!((inner.above, inner.below), (0, 0));
        let a = Kind { block: Block::Paragraph, containers: vec![item(1, 2, false, 1)] };
        let b = Kind { block: Block::Paragraph, containers: vec![item(1, 3, false, 1)] };
        assert!(Look::of(&a, None, Some(&b), 1.0).below < Look::of(&a, None, None, 1.0).below);
    }
}

impl RichView {
    /// Which lines start a top-level block, for the split view's scroll sync.
    pub fn block_start_lines(&self) -> Vec<i32> {
        let buf = self.buf();
        let lines: Vec<(Kind, bool)> = (0..buf.line_count())
            .map(|l| {
                let (s, e) = self.line_bounds(l);
                (self.kind_at_line(l), s == e)
            })
            .collect();
        crate::markdown::scroll::formatted_block_starts(&lines).into_iter().map(|l| l as i32).collect()
    }

    /// Where lines start, as the scroll offset that puts each at the top of the view, the
    /// spacing above it included.
    pub fn offsets_of(&self, lines: &[i32]) -> Vec<f64> {
        let buf = self.buf();
        let Some(adj) = self.vadjustment() else { return vec![] };
        let shift = self.visible_rect().y() as f64 - adj.value();
        lines
            .iter()
            .map(|&l| self.line_yrange(&buf.iter_at_line(l).unwrap_or_else(|| buf.end_iter())).0 as f64 - shift)
            .collect()
    }
}
