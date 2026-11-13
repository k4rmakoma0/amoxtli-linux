//! The source editor: Markdown as plain text, highlighted as it is typed. Only the edited
//! lines are scanned again, unless the edit opens or closes a fenced code block, which
//! changes every line after it.

use crate::markdown::active::{self, ActiveFormats};
use crate::markdown::format::{self, FormatCommand};
use crate::markdown::syntax::{self, Span, SyntaxKind};
use crate::markdown::{continuation, TextEdit};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Marks every line (line break included) after which a fenced code block is open, so an
/// edit can tell whether its first line starts inside a block without rescanning above it.
const FENCED: &str = "fenced";
const HIGHLIGHT_TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "h5", "h6", "marker", "strong", "emphasis", "underline", "strike", "code", "link",
    "quote", "list-marker", FENCED,
];

type Callback = Rc<dyn Fn()>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SourceView {
        /// The lines touched by the last edit, as character offsets once it is applied.
        pub pending: Cell<Option<(i32, i32)>>,
        /// Text replaced by `set_text`, which isn't an edit by the person.
        pub loading: Cell<bool>,
        pub preedit: Cell<bool>,
        pub formats_queued: Cell<bool>,
        pub on_edited: RefCell<Option<Callback>>,
        pub on_formats: RefCell<Option<Callback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SourceView {
        const NAME: &'static str = "AmoxtliSourceView";
        type Type = super::SourceView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for SourceView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for SourceView {}
    impl TextViewImpl for SourceView {}
}

glib::wrapper! {
    pub struct SourceView(ObjectSubclass<imp::SourceView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Default for SourceView {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceView {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("wrap-mode", gtk::WrapMode::WordChar)
            .property("left-margin", 32)
            .property("right-margin", 32)
            .property("top-margin", 28)
            .property("bottom-margin", 64)
            .property("pixels-inside-wrap", 3)
            .property("pixels-below-lines", 3)
            .property("accepts-tab", true)
            .build()
    }

    fn setup(&self) {
        self.add_css_class("md-document");
        self.add_css_class("md-source");
        self.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Markdown"))]);
        let buf = self.buffer();
        let table = buf.tag_table();
        for name in HIGHLIGHT_TAGS.iter().chain(["match", "match-current"].iter()) {
            table.add(&gtk::TextTag::new(Some(name)));
        }
        // search highlights go over the syntax colors
        for name in ["match", "match-current"] {
            if let Some(t) = table.lookup(name) {
                t.set_priority(table.size() - 1);
            }
        }
        self.restyle();

        let weak = self.downgrade();
        buf.connect_insert_text(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, at, text| {
                let start = at.offset();
                view.imp().pending.set(Some((start, start + text.chars().count() as i32)));
            }
        ));
        buf.connect_delete_range(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, start, end| {
                let s = start.offset().min(end.offset());
                view.imp().pending.set(Some((s, s)));
            }
        ));
        buf.connect_changed(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let pending = view.imp().pending.take();
            if view.imp().loading.get() {
                return; // `set_text` styles the whole text once it's in
            }
            if let Some((start, end)) = pending {
                view.highlight_edit(start, end);
            }
            let cb = view.imp().on_edited.borrow().clone();
            if let Some(cb) = cb {
                cb();
            }
            view.queue_formats();
        });
        buf.connect_mark_set(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |buf, _, mark| {
                if mark == &buf.get_insert() || mark == &buf.selection_bound() {
                    view.queue_formats();
                }
            }
        ));
        self.connect_preedit_changed(|view, preedit| view.imp().preedit.set(!preedit.is_empty()));

        // Return continues lists and quotes. Capture phase: the text view would insert the
        // line break itself. While an input method composes, Return confirms it instead.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let plain = !modifiers.intersects(
                    gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK,
                );
                if plain && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter) && view.press_return() {
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(keys);
    }

    // ------------------------------------------------------------ public API

    /// Replaces the whole text, as when a file is opened. It can't be undone.
    pub fn set_text(&self, text: &str) {
        let buf = self.buffer();
        self.imp().loading.set(true);
        buf.begin_irreversible_action();
        buf.set_text(text);
        buf.end_irreversible_action();
        self.imp().loading.set(false);
        self.imp().pending.set(None);
        self.highlight_all();
        buf.place_cursor(&buf.start_iter());
        self.queue_formats();
    }

    pub fn text(&self) -> String {
        let buf = self.buffer();
        buf.text(&buf.start_iter(), &buf.end_iter(), true).into()
    }

    /// Called after each edit by the person (typing, undo, a tool).
    pub fn connect_edited<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_edited.replace(Some(Rc::new(f)));
    }

    /// Called when the formatting at the caret may have changed.
    pub fn connect_formats_changed<F: Fn() + 'static>(&self, f: F) {
        self.imp().on_formats.replace(Some(Rc::new(f)));
    }

    /// The selection as character offsets.
    pub fn selection(&self) -> (i32, i32) {
        let buf = self.buffer();
        match buf.selection_bounds() {
            Some((s, e)) => (s.offset(), e.offset()),
            None => {
                let at = buf.iter_at_mark(&buf.get_insert()).offset();
                (at, at)
            }
        }
    }

    pub fn selected_text(&self) -> String {
        let buf = self.buffer();
        buf.selection_bounds().map(|(s, e)| buf.text(&s, &e, true).to_string()).unwrap_or_default()
    }

    /// Applies an edit as one undoable step, as if it had been typed.
    pub fn apply_edit(&self, edit: &TextEdit) {
        let buf = self.buffer();
        buf.begin_user_action();
        let mut start = buf.iter_at_offset(edit.range.start as i32);
        let mut end = buf.iter_at_offset(edit.range.end as i32);
        if start != end {
            buf.delete(&mut start, &mut end);
        }
        if !edit.replacement.is_empty() {
            let mut at = buf.iter_at_offset(edit.range.start as i32);
            buf.insert(&mut at, &edit.replacement);
        }
        buf.end_user_action();
        let s = buf.iter_at_offset(edit.selection.start as i32);
        let e = buf.iter_at_offset(edit.selection.end as i32);
        buf.select_range(&e, &s);
        self.scroll_mark_onscreen(&buf.get_insert());
    }

    pub fn apply_format(&self, command: &FormatCommand) {
        if !self.is_editable() {
            return;
        }
        let (s, e) = self.selection();
        let edit = format::edit(command, &self.text(), s as usize..e as usize);
        self.apply_edit(&edit);
    }

    /// The formatting at the caret, read from the syntax of its line.
    pub fn active_formats(&self) -> ActiveFormats {
        let buf = self.buffer();
        let (s, e) = self.selection();
        let caret = buf.iter_at_offset(s);
        let line = caret.line();
        let (line_start, line_end) = self.line_bounds(line);
        let content = buf.text(&line_start, &line_end, true).to_string();
        let len = content.chars().count();
        // A selection that leaves the line reaches past its text, so no inline style covers it.
        let local = (s - line_start.offset()) as usize..((e - line_start.offset()) as usize).min(len + 1);
        active::source_formats(&format!("{content}\n"), local, self.starts_in_fence(line))
    }

    /// Moves the caret to a character offset and scrolls it to the top.
    pub fn reveal(&self, offset: i32) {
        let buf = self.buffer();
        let it = buf.iter_at_offset(offset);
        buf.place_cursor(&it);
        let mark = buf.get_insert();
        self.scroll_to_mark(&mark, 0.0, true, 0.0, 0.0);
    }

    /// Colors that depend on the light or dark style and the accent color.
    pub fn restyle(&self) {
        let sm = adw::StyleManager::default();
        let dark = sm.is_dark();
        let fg = self.color();
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.55);
        let accent = sm.accent_color().to_standalone_rgba(dark);
        let high_contrast = sm.is_high_contrast();
        let code_alpha = match (high_contrast, dark) {
            (true, _) => 0.16,
            (false, true) => 0.13,
            (false, false) => 0.075,
        };
        let code_bg = if dark { gdk::RGBA::new(1.0, 1.0, 1.0, code_alpha) } else { gdk::RGBA::new(0.0, 0.0, 0.0, code_alpha) };
        let table = self.buffer().tag_table();
        let tag = |name: &str| table.lookup(name).unwrap_or_else(|| panic!("missing tag {name}"));
        for (level, scale) in [(1, 1.45), (2, 1.3), (3, 1.15), (4, 1.0), (5, 1.0), (6, 1.0)] {
            let t = tag(&format!("h{level}"));
            t.set_scale(scale);
            t.set_weight(700);
        }
        tag("marker").set_foreground_rgba(Some(&dim));
        tag("quote").set_foreground_rgba(Some(&dim));
        tag("strong").set_weight(700);
        tag("emphasis").set_style(pango::Style::Italic);
        tag("underline").set_underline(pango::Underline::Single);
        tag("strike").set_strikethrough(true);
        let code = tag("code");
        code.set_family(Some("monospace"));
        code.set_background_rgba(Some(&code_bg));
        tag("link").set_foreground_rgba(Some(&accent));
        tag("list-marker").set_foreground_rgba(Some(&accent));
        tag("match").set_background_rgba(Some(&gdk::RGBA::new(0.96, 0.83, 0.18, 0.45)));
        tag("match-current").set_background_rgba(Some(&gdk::RGBA::new(1.0, 0.47, 0.0, 0.65)));
    }

    // -------------------------------------------------------------- Return

    /// Return on a list or quote line continues it. Applied as typing, so it undoes like
    /// any other keystroke. `false` leaves Return to the text view.
    pub fn press_return(&self) -> bool {
        if !self.is_editable() || self.imp().preedit.get() {
            return false;
        }
        let buf = self.buffer();
        let (s, e) = self.selection();
        let line = buf.iter_at_offset(s).line();
        let (line_start, line_end) = self.line_bounds(line);
        if e > line_end.offset() {
            return false; // a selection over several lines is replaced as usual
        }
        let content = buf.text(&line_start, &line_end, true).to_string();
        let base = line_start.offset() as usize;
        let local = s as usize - base..e as usize - base;
        let Some(edit) = continuation::edit(&content, local, self.starts_in_fence(line)) else { return false };
        self.apply_edit(&TextEdit {
            range: edit.range.start + base..edit.range.end + base,
            replacement: edit.replacement,
            selection: edit.selection.start + base..edit.selection.end + base,
        });
        true
    }

    // ------------------------------------------------------------ formats

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

    // -------------------------------------------------------- highlighting

    fn line_bounds(&self, line: i32) -> (gtk::TextIter, gtk::TextIter) {
        let buf = self.buffer();
        let start = buf.iter_at_line(line).unwrap_or_else(|| buf.end_iter());
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        (start, end)
    }

    /// Whether a line begins inside a fenced code block, as the last highlighting pass found.
    fn starts_in_fence(&self, line: i32) -> bool {
        if line <= 0 {
            return false;
        }
        let buf = self.buffer();
        let Some(fenced) = buf.tag_table().lookup(FENCED) else { return false };
        let mut prev_end = buf.iter_at_line(line).unwrap_or_else(|| buf.end_iter());
        prev_end.backward_char() && prev_end.has_tag(&fenced)
    }

    fn highlight_all(&self) {
        let lines = self.buffer().line_count();
        self.highlight(0, lines - 1, false, false);
    }

    fn highlight_edit(&self, start: i32, end: i32) {
        let buf = self.buffer();
        let first = buf.iter_at_offset(start).line();
        let last = buf.iter_at_offset(end).line();
        self.highlight(first, last, self.starts_in_fence(first), true);
    }

    /// Styles lines `first..=last`. With `follow_up`, when the edit opened or closed a
    /// code block, the lines after it are styled again too.
    fn highlight(&self, first: i32, last: i32, starts_in_fence: bool, follow_up: bool) {
        let buf = self.buffer();
        let table = buf.tag_table();
        let tags: Vec<gtk::TextTag> = HIGHLIGHT_TAGS.iter().filter_map(|n| table.lookup(n)).collect();
        let fenced = table.lookup(FENCED).expect("fenced tag");
        let mut in_fence = starts_in_fence;
        let mut spans: Vec<Span> = vec![];
        for line in first..=last {
            let (start, content_end) = self.line_bounds(line);
            let mut next = start;
            if !next.forward_line() {
                next = buf.end_iter();
            }
            for t in &tags {
                buf.remove_tag(t, &start, &next);
            }
            let content = buf.text(&start, &content_end, true);
            spans.clear();
            in_fence = syntax::scan_line(&content, in_fence, &mut spans);
            for span in &spans {
                let (Some(s), Some(e)) = (
                    buf.iter_at_line_index(line, span.range.start as i32),
                    buf.iter_at_line_index(line, span.range.end as i32),
                ) else {
                    continue;
                };
                if let Some(tag) = table.lookup(tag_name(span.kind)) {
                    buf.apply_tag(&tag, &s, &e);
                }
            }
            if in_fence {
                buf.apply_tag(&fenced, &start, &next);
            }
        }
        let lines = buf.line_count();
        if follow_up && last + 1 < lines && in_fence != self.next_line_starts_in_fence(last + 1, &fenced) {
            // The edit opened or closed a code block: everything below it changes.
            self.highlight(last + 1, lines - 1, in_fence, false);
        }
    }

    /// What a line was scanned with, worked out from how it was marked: a fence line marked
    /// as open opened a block, so it started outside one, and the opposite for the others.
    fn next_line_starts_in_fence(&self, line: i32, fenced: &gtk::TextTag) -> bool {
        let buf = self.buffer();
        let (start, end) = self.line_bounds(line);
        let marked = start.has_tag(fenced);
        let content = buf.text(&start, &end, true);
        if syntax::is_fence(&content) {
            !marked
        } else {
            marked
        }
    }
}

fn tag_name(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Heading(1) => "h1",
        SyntaxKind::Heading(2) => "h2",
        SyntaxKind::Heading(3) => "h3",
        SyntaxKind::Heading(4) => "h4",
        SyntaxKind::Heading(5) => "h5",
        SyntaxKind::Heading(_) => "h6",
        SyntaxKind::Marker | SyntaxKind::LinkDestination | SyntaxKind::HorizontalRule => "marker",
        SyntaxKind::Strong => "strong",
        SyntaxKind::Emphasis => "emphasis",
        SyntaxKind::Underline => "underline",
        SyntaxKind::Strikethrough => "strike",
        SyntaxKind::InlineCode | SyntaxKind::CodeBlock => "code",
        SyntaxKind::LinkText => "link",
        SyntaxKind::Quote => "quote",
        SyntaxKind::ListMarker => "list-marker",
    }
}

impl SourceView {
    /// Moves the caret without scrolling, as after reloading the text.
    pub fn place_cursor_at(&self, offset: i32) {
        let buf = self.buffer();
        buf.place_cursor(&buf.iter_at_offset(offset));
    }
}
