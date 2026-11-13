//! The formatted editor: the document editable as in a word processor. Markdown syntax never
//! shows. What a paragraph is (its `Kind`) lives in a tag on every character of its line, line
//! break included, and inline styles live in tags too; the editor writes the Markdown from
//! them. Bullets, numbers, quote bars and code backgrounds are drawn, not text.
//!
//! GtkTextBuffer's own undo only records text, not tags, so undoing would drop formatting:
//! the editor keeps its own history of Markdown snapshots instead.

mod convert;
mod draw;
mod keys;
mod language;
pub mod table;
mod tools;

use crate::document::Heading;
use crate::markdown::code::{self, Language, TokenKind};
use crate::markdown::model::{Block, InlineStyle, Kind, Link};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, pango};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

pub const PAGE_PAD: i32 = 32;
const UNDO_LIMIT: usize = 200;
/// Total size of the undo snapshots kept, in bytes.
const UNDO_BUDGET: usize = 64 << 20;

/// Inline style tags, by name.
pub const STYLE_TAGS: [(InlineStyle, &str); 6] = [
    (InlineStyle::STRONG, "strong"),
    (InlineStyle::EMPHASIS, "emphasis"),
    (InlineStyle::UNDERLINE, "underline"),
    (InlineStyle::STRIKETHROUGH, "strike"),
    (InlineStyle::CODE, "code"),
    (InlineStyle::RAW, "raw"),
];

const KIND_PREFIX: &str = "kind:";
const LINK_PREFIX: &str = "link\u{1}";
const LOOK_PREFIX: &str = "look:";
/// A literal leading hash, loaded from Markdown or entered with `\#`, so later typing can't
/// turn its paragraph into a heading.
pub const LITERAL_HASH: &str = "literal-hash";

type Callback = Rc<dyn Fn()>;
type LinkHandler = Rc<dyn Fn(&str)>;

#[derive(Clone, Debug)]
pub struct Snapshot {
    markdown: String,
    caret: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Group {
    #[default]
    None,
    Insert,
    Delete,
    /// Typing in a table cell, `group_caret` telling which.
    Cell,
}

/// The inline style of the character after an iter, from its tags.
pub fn inline_style_at(it: &gtk::TextIter) -> InlineStyle {
    let tags = it.tags();
    let mut style = InlineStyle::NONE;
    for (flag, name) in STYLE_TAGS {
        if tags.iter().any(|t| t.name().as_deref() == Some(name)) {
            style = style.union(flag);
        }
    }
    style
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RichView {
        pub kinds: RefCell<Vec<Kind>>,
        pub kind_index: RefCell<HashMap<Kind, usize>>,
        /// The kind of the empty last line, which has no characters to hold one.
        pub typing_kind: RefCell<Option<Kind>>,
        /// The style typing takes: of the text before the caret, or toggled with no selection.
        pub typing_style: Cell<InlineStyle>,
        pub base_dir: RefCell<Option<gio::File>>,
        /// The editor's own buffer. While a document is built, the view shows a placeholder:
        /// a buffer on screen relays out at every change, which made long documents take minutes.
        pub own: RefCell<Option<gtk::TextBuffer>>,
        /// Widgets on child anchors, to put back when the buffer returns to the view.
        pub anchored: RefCell<Vec<(gtk::TextChildAnchor, gtk::Widget)>>,
        pub front_matter: RefCell<Option<String>>,
        pub tables: RefCell<Vec<(gtk::TextChildAnchor, Rc<table::Table>)>>,
        pub wide: RefCell<Vec<gtk::Widget>>,
        pub last_width: Cell<i32>,
        pub next_id: Cell<u32>,

        // edits in progress
        pub loading: Cell<bool>,
        pub internal: Cell<u32>,
        pub pending: Cell<Option<(i32, i32)>>,
        pub removed_only: Cell<bool>,
        pub typed: RefCell<Option<(i32, String)>>,
        pub pasting: Cell<bool>,
        pub normalizing: Cell<bool>,
        pub preedit: Cell<bool>,

        // undo
        pub undo: RefCell<Vec<Snapshot>>,
        pub redo: RefCell<Vec<Snapshot>>,
        pub group: Cell<Group>,
        pub group_caret: Cell<i32>,
        pub group_time: Cell<Option<Instant>>,

        // drawing caches
        pub markers: RefCell<Option<Vec<draw::LineMarkers>>>,
        pub headings: RefCell<Option<Vec<Heading>>>,

        pub on_edited: RefCell<Option<Callback>>,
        pub on_formats: RefCell<Option<Callback>>,
        pub on_link: RefCell<Option<LinkHandler>>,
        pub formats_queued: Cell<bool>,
        /// The chips drawn last, with the first line of their code block, for clicks.
        pub chips: RefCell<Vec<(gtk::graphene::Rect, i32)>>,
        pub over_chip: Cell<bool>,
        pub language_action: RefCell<Option<gio::SimpleAction>>,
        /// What the context menu was last built for.
        pub menu_key: RefCell<String>,
        pub hovering: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RichView {
        const NAME: &'static str = "AmoxtliRichView";
        type Type = super::RichView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for RichView {}

    impl WidgetImpl for RichView {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if width != self.last_width.get() {
                self.last_width.set(width);
                let obj = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(obj) = obj.upgrade() {
                        obj.fit_children(width);
                    }
                });
            }
        }
    }

    impl TextViewImpl for RichView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            if layer == gtk::TextViewLayer::BelowText {
                self.obj().paint(&snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct RichView(ObjectSubclass<imp::RichView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Default for RichView {
    fn default() -> Self {
        Self::new()
    }
}

impl RichView {
    pub fn new() -> Self {
        let table = gtk::TextTagTable::new();
        let buffer = gtk::TextBuffer::new(Some(&table));
        buffer.set_enable_undo(false);
        // set up once the buffer is in place: GTK sets it after `constructed`
        let view: Self = glib::Object::builder()
            .property("buffer", &buffer)
            .property("wrap-mode", gtk::WrapMode::WordChar)
            .property("left-margin", PAGE_PAD)
            .property("right-margin", PAGE_PAD)
            .property("top-margin", 28)
            .property("bottom-margin", 64)
            .property("pixels-inside-wrap", 3)
            .property("accepts-tab", true)
            .build();
        view.imp().own.replace(Some(buffer));
        view.setup();
        view
    }

    fn setup(&self) {
        self.add_css_class("md-document");
        self.add_css_class("md-rich");
        self.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Document"))]);
        let table = self.buf().tag_table();
        for (_, name) in STYLE_TAGS {
            table.add(&gtk::TextTag::new(Some(name)));
        }
        for name in ["h1", "h2", "h3", "h4", "h5", "h6", "code-line", "raw-line", "quoted", LITERAL_HASH] {
            table.add(&gtk::TextTag::new(Some(name)));
        }
        for kind in TokenKind::ALL {
            table.add(&gtk::TextTag::new(Some(kind.name())));
        }
        // search highlights last, above everything
        for name in ["match", "match-current"] {
            table.add(&gtk::TextTag::new(Some(name)));
        }
        self.restyle_tags();
        self.connect_buffer_signals();
        self.connect_preedit_changed(|view, preedit| view.imp().preedit.set(!preedit.is_empty()));
        // Text from elsewhere comes in plain, taking the style of where it lands. GTK would
        // paste text copied in this editor with its tags, list items and all.
        self.connect_paste_clipboard(|view| {
            view.stop_signal_emission_by_name("paste-clipboard");
            let weak = view.downgrade();
            view.clipboard().read_text_async(gio::Cancellable::NONE, move |text| {
                let (Some(view), Ok(Some(text))) = (weak.upgrade(), text) else { return };
                if !view.is_editable() {
                    return;
                }
                view.imp().pasting.set(true);
                let buf = view.buffer();
                buf.begin_user_action();
                buf.delete_selection(true, true);
                buf.insert_interactive_at_cursor(&text, true);
                buf.end_user_action();
                view.scroll_mark_onscreen(&buf.get_insert());
            });
        });
        self.setup_keys();
        self.setup_pointer();
        self.setup_language();
    }

    /// The editor's buffer, even while the view shows a placeholder.
    pub fn buf(&self) -> gtk::TextBuffer {
        self.imp().own.borrow().clone().unwrap_or_else(|| self.buffer())
    }

    /// Runs a change of the whole buffer away from the view, then shows it again with its
    /// embedded widgets, caret and scroll position.
    pub fn detached(&self, f: impl FnOnce()) {
        let buf = self.buf();
        let caret = buf.iter_at_mark(&buf.get_insert()).offset();
        let scroll = self.vadjustment().map(|a| a.value());
        self.set_buffer(Some(&gtk::TextBuffer::new(None)));
        f();
        self.set_buffer(Some(&buf));
        let anchored: Vec<_> = self.imp().anchored.borrow().iter().filter(|(a, _)| !a.is_deleted()).cloned().collect();
        for (anchor, widget) in &anchored {
            if widget.parent().is_none() {
                self.add_child_at_anchor(widget, anchor);
            }
        }
        self.imp().anchored.replace(anchored);
        buf.place_cursor(&buf.iter_at_offset(caret));
        if let (Some(adj), Some(value)) = (self.vadjustment(), scroll) {
            glib::idle_add_local_once(move || adj.set_value(value));
        }
    }

    // ------------------------------------------------------------- public API

    pub fn connect_edited<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_edited.replace(Some(Rc::new(f)));
    }

    pub fn connect_formats_changed<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_formats.replace(Some(Rc::new(f)));
    }

    pub fn connect_link_activated<F: Fn(&str) + 'static>(&self, f: F) {
        self.imp().on_link.replace(Some(Rc::new(f)));
    }

    fn activate_link(&self, url: &str) {
        let handler = self.imp().on_link.borrow().clone();
        if let Some(h) = handler {
            h(url);
        }
    }

    pub fn make_id(&self) -> u32 {
        // Far above the ids of a parsed document, so the two never collide.
        let id = self.imp().next_id.get().max(1_000_000) + 1;
        self.imp().next_id.set(id);
        id
    }

    // ------------------------------------------------------------- tags

    /// Adds a tag. Search highlights stay on top without raising them: the tags added later
    /// set no background. Changing a priority restyles the whole text, and with one tag per
    /// list item that made opening long documents quadratic.
    fn add_tag(&self, tag: &gtk::TextTag) {
        self.buf().tag_table().add(tag);
    }

    fn tag(&self, name: &str) -> gtk::TextTag {
        self.buf().tag_table().lookup(name).unwrap_or_else(|| panic!("missing tag {name}"))
    }

    pub fn kind_tag(&self, kind: &Kind) -> gtk::TextTag {
        let imp = self.imp();
        let index = imp.kind_index.borrow().get(kind).copied();
        let index = index.unwrap_or_else(|| {
            let mut kinds = imp.kinds.borrow_mut();
            kinds.push(kind.clone());
            imp.kind_index.borrow_mut().insert(kind.clone(), kinds.len() - 1);
            kinds.len() - 1
        });
        let name = format!("{KIND_PREFIX}{index}");
        match self.buf().tag_table().lookup(&name) {
            Some(t) => t,
            None => {
                let t = gtk::TextTag::new(Some(&name));
                self.add_tag(&t);
                t
            }
        }
    }

    fn kind_of_tag(&self, tag: &gtk::TextTag) -> Option<Kind> {
        let name = tag.name()?;
        let index: usize = name.strip_prefix(KIND_PREFIX)?.parse().ok()?;
        self.imp().kinds.borrow().get(index).cloned()
    }

    pub fn link_tag(&self, link: &Link) -> gtk::TextTag {
        let name = format!("{LINK_PREFIX}{}\u{1}{}", link.destination, link.title);
        if let Some(t) = self.buf().tag_table().lookup(&name) {
            return t;
        }
        let t = gtk::TextTag::new(Some(&name));
        self.style_link_tag(&t);
        self.add_tag(&t);
        t
    }

    pub fn link_of_tag(tag: &gtk::TextTag) -> Option<Link> {
        let name = tag.name()?;
        let rest = name.strip_prefix(LINK_PREFIX)?;
        let (destination, title) = rest.split_once('\u{1}').unwrap_or((rest, ""));
        Some(Link { destination: destination.to_owned(), title: title.to_owned() })
    }

    fn style_link_tag(&self, tag: &gtk::TextTag) {
        let sm = adw::StyleManager::default();
        tag.set_foreground_rgba(Some(&sm.accent_color().to_standalone_rgba(sm.is_dark())));
        tag.set_underline(pango::Underline::Single);
    }

    /// The tags of a line's look, derived from its kind and neighbors, by their settings.
    fn look_tag(&self, look: &draw::Look) -> gtk::TextTag {
        let name = format!("{LOOK_PREFIX}{look:?}");
        if let Some(t) = self.buf().tag_table().lookup(&name) {
            return t;
        }
        let t = gtk::TextTag::builder()
            .name(&name)
            .left_margin(look.left)
            .right_margin(look.right)
            .pixels_above_lines(look.above)
            .pixels_below_lines(look.below)
            .build();
        if look.code {
            t.set_wrap_mode(gtk::WrapMode::Char);
        }
        self.add_tag(&t);
        t
    }

    /// Colors and sizes that depend on the style and the text size.
    pub fn restyle_tags(&self) {
        let sm = adw::StyleManager::default();
        let dark = sm.is_dark();
        let fg = self.color();
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.62);
        let code_bg = draw::surface(draw::Surface::Code, dark, sm.is_high_contrast());
        let scales = [2.0, 1.69, 1.31, 1.15, 1.0, 0.85];
        for (i, scale) in scales.iter().enumerate() {
            let t = self.tag(&format!("h{}", i + 1));
            t.set_scale(*scale);
            t.set_weight(if i < 2 { 800 } else { 700 });
        }
        self.tag("strong").set_weight(700);
        self.tag("emphasis").set_style(pango::Style::Italic);
        self.tag("underline").set_underline(pango::Underline::Single);
        self.tag("strike").set_strikethrough(true);
        let code = self.tag("code");
        code.set_family(Some("monospace"));
        code.set_background_rgba(Some(&code_bg));
        let raw = self.tag("raw");
        raw.set_foreground_rgba(Some(&dim));
        raw.set_family(Some("monospace"));
        self.tag("code-line").set_family(Some("monospace"));
        let raw_line = self.tag("raw-line");
        raw_line.set_family(Some("monospace"));
        raw_line.set_foreground_rgba(Some(&dim));
        self.tag("quoted").set_foreground_rgba(Some(&dim));
        for kind in TokenKind::ALL {
            let color = kind.color(dark).and_then(|c| gdk::RGBA::parse(c).ok()).unwrap_or(dim);
            self.tag(kind.name()).set_foreground_rgba(Some(&color));
        }
        self.tag("match").set_background_rgba(Some(&gdk::RGBA::new(0.96, 0.83, 0.18, 0.45)));
        self.tag("match-current").set_background_rgba(Some(&gdk::RGBA::new(1.0, 0.47, 0.0, 0.65)));
        let table = self.buf().tag_table();
        let mut links = vec![];
        table.foreach(|t| {
            if t.name().is_some_and(|n| n.starts_with(LINK_PREFIX)) {
                links.push(t.clone());
            }
        });
        for t in links {
            self.style_link_tag(&t);
        }
        for (_, t) in self.imp().tables.borrow().iter() {
            t.restyle();
        }
    }

    // ------------------------------------------------------------- lines and kinds

    pub fn line_bounds(&self, line: i32) -> (gtk::TextIter, gtk::TextIter) {
        let buf = self.buf();
        let start = buf.iter_at_line(line).unwrap_or_else(|| buf.end_iter());
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        (start, end)
    }

    /// The start of the next line, or the end of the text: the line with its line break.
    fn line_end_with_break(&self, line: i32) -> gtk::TextIter {
        let buf = self.buf();
        let mut next = buf.iter_at_line(line).unwrap_or_else(|| buf.end_iter());
        if !next.forward_line() {
            next = buf.end_iter();
        }
        next
    }

    fn kind_on_chars(&self, line: i32) -> Option<Kind> {
        let (mut it, _) = self.line_bounds(line);
        let end = self.line_end_with_break(line);
        while it < end {
            if let Some(kind) = it.tags().iter().find_map(|t| self.kind_of_tag(t)) {
                return Some(kind);
            }
            it.forward_char();
        }
        None
    }

    /// What a line is: the kind on its characters. The empty last line has none, so it is
    /// the kind kept for typing, else it continues the text before it, unless that is a
    /// block that can't be typed into.
    pub fn kind_at_line(&self, line: i32) -> Kind {
        if let Some(kind) = self.kind_on_chars(line) {
            return kind;
        }
        if let Some(kind) = self.imp().typing_kind.borrow().clone() {
            if self.is_empty_last_line(line) {
                return kind;
            }
        }
        if line > 0 {
            if let Some(previous) = self.kind_on_chars(line - 1) {
                return if previous.block.is_text() { previous } else { Kind::body().with_containers(previous.containers) };
            }
        }
        Kind::body()
    }

    pub fn is_empty_last_line(&self, line: i32) -> bool {
        let buf = self.buf();
        line == buf.line_count() - 1 && self.line_bounds(line).0.is_end()
    }

    pub fn caret_line(&self) -> i32 {
        let buf = self.buf();
        buf.iter_at_mark(&buf.get_insert()).line()
    }

    pub fn kind_at_caret(&self) -> Kind {
        self.kind_at_line(self.caret_line())
    }

    /// Sets a line's kind, even the empty last one's, and restyles it with its neighbors.
    pub fn set_line_kind(&self, line: i32, kind: &Kind) {
        if self.is_empty_last_line(line) {
            self.imp().typing_kind.replace(Some(kind.clone()));
        } else {
            let (start, _) = self.line_bounds(line);
            let end = self.line_end_with_break(line);
            self.remove_kind_tags(&start, &end);
            self.buf().apply_tag(&self.kind_tag(kind), &start, &end);
        }
        self.restyle_lines(line - 1, line + 1);
        self.changed_layout();
    }

    fn remove_kind_tags(&self, start: &gtk::TextIter, end: &gtk::TextIter) {
        let buf = self.buf();
        let mut tags = vec![];
        let mut it = *start;
        while it < *end {
            for t in it.tags() {
                if t.name().is_some_and(|n| n.starts_with(KIND_PREFIX)) && !tags.contains(&t) {
                    tags.push(t);
                }
            }
            if !it.forward_to_tag_toggle(None::<&gtk::TextTag>) || it > *end {
                break;
            }
        }
        for t in tags {
            buf.remove_tag(&t, start, end);
        }
    }

    /// Code and raw lines ignore inline styles.
    fn strips_inline(kind: &Kind) -> bool {
        matches!(kind.block, Block::Code { .. } | Block::Raw { .. })
    }

    // ------------------------------------------------------------- normalizing

    /// Makes every line in `first..=last` uniform: it takes the kind of its first character
    /// with one, and is styled again with its neighbors, whose spacing depends on it.
    pub fn restyle_lines(&self, first: i32, last: i32) {
        let buf = self.buf();
        let count = buf.line_count();
        let first = first.max(0);
        let last = last.min(count - 1);
        if first > last {
            return;
        }
        let scale = self.scale();
        let empty_next = self.empty_last_kind();
        let kinds: Vec<Kind> = (first - 1..=last + 1)
            .map(|l| if l < 0 || l >= count { Kind::body() } else { self.kind_at_line(l) })
            .collect();
        for line in first..=last {
            let i = (line - first + 1) as usize;
            let kind = &kinds[i];
            let previous = (line > 0).then(|| &kinds[i - 1]);
            let next = if line + 1 >= count {
                None
            } else if self.is_empty_last_line(line + 1) {
                empty_next.as_ref()
            } else {
                Some(&kinds[i + 1])
            };
            let (start, content_end) = self.line_bounds(line);
            let end = self.line_end_with_break(line);
            // every derived tag goes, then comes back from the kind
            let mut stale = vec![];
            let mut it = start;
            loop {
                for t in it.tags() {
                    let name = t.name().map(|n| n.to_string()).unwrap_or_default();
                    let derived = name.starts_with(LOOK_PREFIX)
                        || name.starts_with(KIND_PREFIX)
                        || ["h1", "h2", "h3", "h4", "h5", "h6", "code-line", "raw-line", "quoted"].contains(&name.as_str());
                    if derived && !stale.contains(&t) {
                        stale.push(t);
                    }
                }
                if !it.forward_to_tag_toggle(None::<&gtk::TextTag>) || it >= end {
                    break;
                }
            }
            for t in &stale {
                buf.remove_tag(t, &start, &end);
            }
            if start == end {
                continue; // the empty last line: drawn from the typing kind
            }
            for tag in self.line_tags(kind, previous, next, scale) {
                buf.apply_tag(&tag, &start, &end);
            }
            if Self::strips_inline(kind) {
                for (_, name) in STYLE_TAGS {
                    buf.remove_tag(&self.tag(name), &start, &content_end);
                }
                self.remove_link_tags(&start, &content_end);
            }
        }
        self.color_code_blocks(first, last);
    }

    /// The tags a line carries: its kind, and its look derived from the kind and the neighbors.
    pub(super) fn line_tags(&self, kind: &Kind, previous: Option<&Kind>, next: Option<&Kind>, scale: f64) -> Vec<gtk::TextTag> {
        let mut tags = vec![self.kind_tag(kind), self.look_tag(&draw::Look::of(kind, previous, next, scale))];
        match &kind.block {
            Block::Heading { level, .. } => tags.push(self.tag(&format!("h{}", level.clamp(&1, &6)))),
            Block::Code { .. } => tags.push(self.tag("code-line")),
            Block::Raw { .. } => tags.push(self.tag("raw-line")),
            _ => {}
        }
        if kind.is_quoted() {
            tags.push(self.tag("quoted"));
        }
        tags
    }

    /// Colors the code blocks lines `first..=last` touch, each one whole: a comment or a
    /// string may span lines.
    pub fn color_code_blocks(&self, first: i32, last: i32) {
        let count = self.buf().line_count();
        let mut line = first.clamp(0, count - 1);
        while line > 0 && draw::same_code(&self.kind_at_line(line), Some(&self.kind_at_line(line - 1))) {
            line -= 1;
        }
        while line < count && line <= last {
            let kind = self.kind_at_line(line);
            let Block::Code { info, .. } = &kind.block else {
                line += 1;
                continue;
            };
            let mut end = line;
            while end + 1 < count && draw::same_code(&kind, Some(&self.kind_at_line(end + 1))) {
                end += 1;
            }
            self.color_block(line, end, info.as_deref());
            line = end + 1;
        }
    }

    fn color_block(&self, first: i32, last: i32, info: Option<&str>) {
        let buf = self.buf();
        let (start, _) = self.line_bounds(first);
        let (_, end) = self.line_bounds(last);
        for kind in TokenKind::ALL {
            buf.remove_tag(&self.tag(kind.name()), &start, &end);
        }
        let Some(language) = info.and_then(Language::from_info) else { return };
        let base = start.offset();
        for token in code::tokens(&buf.text(&start, &end, false), language) {
            let (s, e) = (buf.iter_at_offset(base + token.range.start as i32), buf.iter_at_offset(base + token.range.end as i32));
            buf.apply_tag(&self.tag(token.kind.name()), &s, &e);
        }
    }

    /// The kind of the empty last line while the caret is on it, when it isn't body text. It
    /// has no characters, but it shows its marker and the line above is spaced against it.
    pub fn empty_last_kind(&self) -> Option<Kind> {
        let line = self.caret_line();
        if !self.is_empty_last_line(line) {
            return None;
        }
        self.imp().typing_kind.borrow().clone().filter(|k| *k != Kind::body())
    }

    pub fn remove_link_tags(&self, start: &gtk::TextIter, end: &gtk::TextIter) {
        let buf = self.buf();
        let mut tags = vec![];
        buf.tag_table().foreach(|t| {
            if t.name().is_some_and(|n| n.starts_with(LINK_PREFIX)) {
                tags.push(t.clone());
            }
        });
        for t in tags {
            buf.remove_tag(&t, start, end);
        }
    }

    pub fn restyle_all(&self) {
        self.restyle_tags();
        self.detached(|| {
            let lines = self.buf().line_count();
            self.restyle_lines(0, lines - 1);
        });
        self.changed_layout();
    }

    /// Text size relative to the standard one.
    pub fn scale(&self) -> f64 {
        crate::application::current_size().scale()
    }

    /// Markers and headings depend on every line: worked out again when next needed.
    fn changed_layout(&self) {
        self.imp().markers.replace(None);
        self.imp().headings.replace(None);
        self.queue_draw();
        self.queue_formats();
    }

    // ------------------------------------------------------------- edits

    fn connect_buffer_signals(&self) {
        let buf = self.buf();
        buf.connect_insert_text(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, at, text| view.before_change(at.offset(), at.offset(), Some(text))
        ));
        buf.connect_delete_range(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, start, end| view.before_change(start.offset(), end.offset(), None)
        ));
        let weak = self.downgrade();
        buf.connect_local("insert-text", true, move |values| {
            let view = weak.upgrade()?;
            let at = values[1].get::<gtk::TextIter>().ok()?;
            let text = values[2].get::<String>().ok()?;
            view.after_insert(at.offset() - text.chars().count() as i32, at.offset(), &text);
            None
        });
        let weak = self.downgrade();
        buf.connect_local("delete-range", true, move |values| {
            let view = weak.upgrade()?;
            let at = values[1].get::<gtk::TextIter>().ok()?;
            view.note_pending(at.offset(), at.offset());
            None
        });
        buf.connect_end_user_action(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.finish_edit()
        ));
        buf.connect_mark_set(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |buf, _, mark| {
                if mark == &buf.get_insert() {
                    view.caret_moved();
                }
                if mark == &buf.get_insert() || mark == &buf.selection_bound() {
                    view.queue_formats();
                }
            }
        ));
    }

    fn note_pending(&self, start: i32, end: i32) {
        let imp = self.imp();
        let merged = match imp.pending.get() {
            Some((s, e)) => (s.min(start), e.max(end)),
            None => (start, end),
        };
        imp.pending.set(Some(merged));
    }

    /// Before the text changes: opens an undo step unless typing continues the last one.
    fn before_change(&self, start: i32, end: i32, inserted: Option<&str>) {
        let imp = self.imp();
        if imp.loading.get() {
            return;
        }
        if inserted.is_none() {
            imp.removed_only.set(imp.pending.get().is_none() || imp.removed_only.get());
        } else {
            imp.removed_only.set(false);
        }
        if imp.internal.get() > 0 {
            return;
        }
        let group = match inserted {
            Some(t) if !t.contains(['\n', '\u{2028}']) && t.chars().count() <= 4 => Group::Insert,
            None if end - start <= 1 => Group::Delete,
            _ => Group::None,
        };
        let continues = group != Group::None
            && imp.group.get() == group
            && imp.group_caret.get() == if group == Group::Delete { end } else { start }
            && imp.group_time.get().is_some_and(|t| t.elapsed().as_secs_f64() < 2.0);
        if !continues {
            self.checkpoint();
        }
        imp.group.set(group);
        imp.group_time.set(Some(Instant::now()));
    }

    fn after_insert(&self, start: i32, end: i32, text: &str) {
        let imp = self.imp();
        if imp.loading.get() {
            return;
        }
        self.note_pending(start, end);
        if imp.internal.get() == 0 {
            imp.typed.replace(Some((start, text.to_owned())));
            imp.group_caret.set(end);
        }
        // Inserted text takes the kind of its line and the style typing has.
        let buf = self.buf();
        let (s, e) = (buf.iter_at_offset(start), buf.iter_at_offset(end));
        let line = s.line();
        // On a line with no kind on its characters yet, the empty last line, typing takes the
        // kind kept for it, which from then on lives on the characters.
        let kind = match self.kind_on_chars_outside(line, start, end) {
            Some(kind) => kind,
            None => match imp.typing_kind.take() {
                Some(kind) => kind,
                None => self.kind_at_line(line),
            },
        };
        buf.apply_tag(&self.kind_tag(&kind), &s, &e);
        if imp.internal.get() == 0 && !Self::strips_inline(&kind) {
            let style = imp.typing_style.get();
            for (flag, name) in STYLE_TAGS {
                if style.contains(flag) {
                    buf.apply_tag(&self.tag(name), &s, &e);
                }
            }
        }
    }

    /// The kind on a line's characters, other than the ones just inserted.
    fn kind_on_chars_outside(&self, line: i32, start: i32, end: i32) -> Option<Kind> {
        let (mut it, _) = self.line_bounds(line);
        let stop = self.line_end_with_break(line);
        while it < stop {
            if it.offset() < start || it.offset() >= end {
                if let Some(kind) = it.tags().iter().find_map(|t| self.kind_of_tag(t)) {
                    return Some(kind);
                }
            }
            it.forward_char();
        }
        None
    }

    /// Once an edit is done: lines are made uniform, then the rules that depend on what was
    /// typed or deleted apply, and the window hears of it.
    fn finish_edit(&self) {
        let imp = self.imp();
        if imp.loading.get() || imp.normalizing.get() || imp.internal.get() > 0 {
            return;
        }
        let Some((start, end)) = imp.pending.take() else { return };
        imp.normalizing.set(true);
        let buf = self.buf();
        let first = buf.iter_at_offset(start).line();
        let last = buf.iter_at_offset(end).line();
        self.restyle_lines(first - 1, last + 1);
        let typed = imp.typed.take();
        let pasted = imp.pasting.replace(false);
        if let Some((at, text)) = typed.filter(|_| !pasted) {
            self.heading_input(at, &text);
        }
        if imp.removed_only.replace(false) {
            self.clear_empty_element_at_caret();
        }
        // the rules restyle what they change
        imp.pending.set(None);
        imp.normalizing.set(false);
        self.update_typing_style();
        self.changed_layout();
        self.notify_edited();
    }

    fn notify_edited(&self) {
        let cb = self.imp().on_edited.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// Runs a change of the editor's own (a tool, a key) as one undo step.
    pub fn edit<R>(&self, f: impl FnOnce() -> R) -> R {
        let imp = self.imp();
        self.checkpoint();
        imp.group.set(Group::None);
        let buf = self.buf();
        imp.internal.set(imp.internal.get() + 1);
        buf.begin_user_action();
        let result = f();
        buf.end_user_action();
        imp.internal.set(imp.internal.get() - 1);
        if let Some((start, end)) = imp.pending.take() {
            let first = buf.iter_at_offset(start).line();
            let last = buf.iter_at_offset(end).line();
            self.restyle_lines(first - 1, last + 1);
        }
        imp.removed_only.set(false);
        imp.typed.replace(None);
        self.update_typing_style();
        self.changed_layout();
        self.notify_edited();
        result
    }

    // ------------------------------------------------------------- typing

    fn caret_moved(&self) {
        let imp = self.imp();
        if imp.internal.get() > 0 || imp.loading.get() {
            return;
        }
        let buf = self.buf();
        let caret = buf.iter_at_mark(&buf.get_insert()).offset();
        if caret != imp.group_caret.get() {
            imp.group.set(Group::None); // a moved caret starts a new undo step
        }
        self.update_context_menu();
        if imp.pending.get().is_none() {
            self.update_typing_style();
            // the empty last line forgets its kind once the caret leaves it
            if !self.is_empty_last_line(self.caret_line()) && imp.typing_kind.borrow().is_some() {
                imp.typing_kind.replace(None);
                let last = self.buf().line_count() - 1;
                self.restyle_lines(last - 1, last);
                self.changed_layout();
            }
        }
    }

    /// Typing continues the style of the text just before the caret, on its line.
    pub fn update_typing_style(&self) {
        let buf = self.buf();
        let caret = buf.iter_at_mark(&buf.get_insert());
        let style = if caret.starts_line() {
            InlineStyle::NONE
        } else {
            let mut before = caret;
            before.backward_char();
            self.style_at(&before)
        };
        self.imp().typing_style.set(style);
    }

    pub fn style_at(&self, it: &gtk::TextIter) -> InlineStyle {
        inline_style_at(it)
    }

    pub fn link_at_iter(it: &gtk::TextIter) -> Option<Link> {
        it.tags().iter().find_map(Self::link_of_tag)
    }

    fn queue_formats(&self) {
        let imp = self.imp();
        if imp.formats_queued.replace(true) {
            return;
        }
        let weak = self.downgrade();
        glib::idle_add_local_once(move || {
            let Some(view) = weak.upgrade() else { return };
            view.imp().formats_queued.set(false);
            let cb = view.imp().on_formats.borrow().clone();
            if let Some(cb) = cb {
                cb();
            }
        });
    }

    // ------------------------------------------------------------- undo

    fn snapshot(&self) -> Snapshot {
        let buf = self.buf();
        Snapshot { markdown: self.markdown(), caret: buf.iter_at_mark(&buf.get_insert()).offset() }
    }

    /// Remembers the document as it is, before a change.
    fn checkpoint(&self) {
        let imp = self.imp();
        let snapshot = self.snapshot();
        let mut undo = imp.undo.borrow_mut();
        if undo.last().is_some_and(|s| s.markdown == snapshot.markdown) {
            return;
        }
        undo.push(snapshot);
        let mut total: usize = undo.iter().map(|s| s.markdown.len()).sum();
        while undo.len() > UNDO_LIMIT || (total > UNDO_BUDGET && undo.len() > 1) {
            total -= undo.remove(0).markdown.len();
        }
        imp.redo.borrow_mut().clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.imp().undo.borrow().is_empty()
    }

    pub fn undo(&self) {
        let imp = self.imp();
        let Some(previous) = imp.undo.borrow_mut().pop() else { return };
        let current = self.snapshot();
        if current.markdown == previous.markdown {
            // nothing changed since that step: go one further
            imp.redo.borrow_mut().push(current);
            return self.undo();
        }
        imp.redo.borrow_mut().push(current);
        self.restore(&previous);
    }

    pub fn redo(&self) {
        let imp = self.imp();
        let Some(next) = imp.redo.borrow_mut().pop() else { return };
        imp.undo.borrow_mut().push(self.snapshot());
        self.restore(&next);
    }

    fn restore(&self, snapshot: &Snapshot) {
        let base = self.imp().base_dir.borrow().clone();
        let (undo, redo) = (self.imp().undo.take(), self.imp().redo.take());
        self.show_markdown(&snapshot.markdown, base);
        self.imp().undo.replace(undo);
        self.imp().redo.replace(redo);
        let buf = self.buf();
        buf.place_cursor(&buf.iter_at_offset(snapshot.caret));
        self.scroll_mark_onscreen(&buf.get_insert());
        self.imp().group.set(Group::None);
        self.notify_edited();
    }

    /// The undo history no longer matches a text that changed elsewhere.
    pub fn forget_history(&self) {
        self.imp().undo.borrow_mut().clear();
        self.imp().redo.borrow_mut().clear();
        self.imp().group.set(Group::None);
    }

    // ------------------------------------------------------------- layout

    fn fit_children(&self, width: i32) {
        let base = width - 2 * PAGE_PAD;
        for widget in self.imp().wide.borrow().iter() {
            let avail = (base - 2).max(32);
            if let Some(fb) = widget.downcast_ref::<crate::widgets::FitBox>() {
                fb.set_avail(avail);
            } else if let Some(img) = widget.first_child().and_then(|c| c.downcast::<crate::widgets::FitImage>().ok()) {
                img.fit(avail);
            }
        }
    }

    fn refit(&self) {
        self.imp().last_width.set(-1);
        self.queue_allocate();
    }
}
