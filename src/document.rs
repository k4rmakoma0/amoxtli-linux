//! Native Markdown rendering into a GtkTextView.
//!
//! No web engine: text is laid out by Pango inside a GtkTextView, tables are
//! GtkGrids of labels, images are GdkTextures and code/quote decorations are
//! painted with GtkSnapshot under the text.

use crate::i18n::tr;
use crate::widgets::{FitBox, FitImage};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, pango};
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

const LIST_INDENT: i32 = 28;
const QUOTE_INDENT: i32 = 22;
const CODE_PAD: i32 = 16;
const PAGE_PAD: i32 = 32; // block tags replace the view margin, so they carry the page padding
const BULLETS: [&str; 3] = ["•", "◦", "▪"];

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

#[derive(Clone, Debug)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub slug: String,
    pub mark: gtk::TextMark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecorKind {
    Code,
    Quote(Option<BlockQuoteKind>),
}

#[derive(Clone)]
pub struct Decor {
    pub kind: DecorKind,
    pub start: gtk::TextMark,
    pub end: gtk::TextMark,
    pub left: i32,
}

type LinkHandler = Rc<dyn Fn(&str)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DocumentView {
        pub headings: RefCell<Vec<Heading>>,
        pub anchors: RefCell<Vec<(String, gtk::TextMark)>>,
        pub links: RefCell<HashMap<gtk::TextTag, String>>,
        pub wide: RefCell<Vec<(gtk::Widget, i32)>>,
        pub decor: RefCell<Vec<Decor>>,
        pub base_dir: RefCell<Option<gio::File>>,
        pub link_handler: RefCell<Option<LinkHandler>>,
        pub last_width: Cell<i32>,
        pub hovering: Cell<bool>,
        pub code_bg: Cell<Option<gdk::RGBA>>,
        pub quote_bar: Cell<Option<gdk::RGBA>>,
        pub dark: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DocumentView {
        const NAME: &'static str = "AmoxtliDocumentView";
        type Type = super::DocumentView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for DocumentView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for DocumentView {
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

    impl TextViewImpl for DocumentView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            if layer == gtk::TextViewLayer::BelowText {
                self.obj().paint_decorations(&snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct DocumentView(ObjectSubclass<imp::DocumentView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Default for DocumentView {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentView {
    pub fn new() -> Self {
        let view: Self = glib::Object::builder()
            .property("editable", false)
            .property("cursor-visible", false)
            .property("wrap-mode", gtk::WrapMode::WordChar)
            .property("left-margin", PAGE_PAD)
            .property("right-margin", PAGE_PAD)
            .property("top-margin", 28)
            .property("bottom-margin", 64)
            .property("pixels-inside-wrap", 3)
            .build();
        view.add_css_class("md-document");
        view
    }

    fn setup(&self) {
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        click.connect_released(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _, x, y| {
                if view.buffer().has_selection() {
                    return;
                }
                if let Some(url) = view.link_at(x, y) {
                    view.activate_link(&url);
                }
            }
        ));
        self.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, x, y| {
                let hovering = view.link_at(x, y).is_some();
                if hovering != view.imp().hovering.get() {
                    view.imp().hovering.set(hovering);
                    view.set_cursor_from_name(Some(if hovering { "pointer" } else { "text" }));
                }
            }
        ));
        self.add_controller(motion);

        self.set_has_tooltip(true);
        self.connect_query_tooltip(|view, x, y, keyboard, tooltip| {
            if keyboard {
                return false;
            }
            match view.link_at(x as f64, y as f64) {
                Some(url) => {
                    tooltip.set_text(Some(&url));
                    true
                }
                None => false,
            }
        });
    }

    // ------------------------------------------------------------ public API

    pub fn connect_link_activated<F: Fn(&str) + 'static>(&self, f: F) {
        self.imp().link_handler.replace(Some(Rc::new(f)));
    }

    fn activate_link(&self, url: &str) {
        let handler = self.imp().link_handler.borrow().clone();
        if let Some(h) = handler {
            h(url);
        }
    }

    pub fn headings(&self) -> Vec<Heading> {
        self.imp().headings.borrow().clone()
    }

    pub fn find_anchor(&self, slug: &str) -> Option<gtk::TextMark> {
        let slug = percent_decode(slug).to_lowercase();
        if let Some(h) = self.imp().headings.borrow().iter().find(|h| h.slug == slug) {
            return Some(h.mark.clone());
        }
        self.imp().anchors.borrow().iter().find(|(s, _)| *s == slug).map(|(_, m)| m.clone())
    }

    pub fn find_heading(&self, slug: &str) -> Option<Heading> {
        let slug = percent_decode(slug).to_lowercase();
        self.imp().headings.borrow().iter().find(|h| h.slug == slug).cloned()
    }

    pub fn heading_y(&self, heading: &Heading) -> i32 {
        let it = self.buffer().iter_at_mark(&heading.mark);
        self.iter_location(&it).y()
    }

    /// Words of prose between two iters (whole document by default), skipping code blocks.
    pub fn count_words(&self, range: Option<(gtk::TextIter, gtk::TextIter)>) -> u64 {
        let buf = self.buffer();
        let (start, end) = range.unwrap_or_else(|| buf.bounds());
        let (lo, hi) = (start.offset(), end.offset());
        let mut total = count_words(&buf.text(&start, &end, false)) as i64;
        for d in self.imp().decor.borrow().iter() {
            if d.kind != DecorKind::Code {
                continue;
            }
            let cs = lo.max(buf.iter_at_mark(&d.start).offset());
            let ce = hi.min(buf.iter_at_mark(&d.end).offset());
            if cs < ce {
                let code = buf.text(&buf.iter_at_offset(cs), &buf.iter_at_offset(ce), false);
                total -= count_words(&code) as i64;
            }
        }
        total.max(0) as u64
    }

    pub fn render(&self, source: &str, base_dir: Option<gio::File>) {
        let imp = self.imp();
        imp.base_dir.replace(base_dir);
        imp.links.borrow_mut().clear();
        imp.wide.borrow_mut().clear();
        imp.decor.borrow_mut().clear();
        imp.headings.borrow_mut().clear();
        imp.anchors.borrow_mut().clear();

        let buf = gtk::TextBuffer::new(None);
        self.set_buffer(Some(&buf)); // before anchoring child widgets
        let mut r = Renderer::new(self, &buf);
        r.run(source);

        // drop the trailing blank line
        let mut end = buf.end_iter();
        let mut start = end;
        if start.backward_char() && start.char() == '\n' {
            buf.delete(&mut start, &mut end);
        }
        imp.last_width.set(-1);
        self.queue_allocate();
    }

    // -------------------------------------------------------------- layout

    fn fit_children(&self, width: i32) {
        let base = width - 2 * PAGE_PAD;
        for (widget, inset) in self.imp().wide.borrow().iter() {
            let avail = (base - inset - 2).max(32);
            if let Some(fb) = widget.downcast_ref::<FitBox>() {
                fb.set_avail(avail);
            } else if let Some(img) = widget.first_child().and_then(|c| c.downcast::<FitImage>().ok()) {
                img.fit(avail);
            }
        }
    }

    fn paint_decorations(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let decor = imp.decor.borrow();
        if decor.is_empty() {
            return;
        }
        let buf = self.buffer();
        let width = self.width();
        let visible = self.visible_rect();
        let code_bg = imp.code_bg.get().unwrap_or(gdk::RGBA::new(0.5, 0.5, 0.5, 0.1));
        let bar = imp.quote_bar.get().unwrap_or(gdk::RGBA::new(0.5, 0.5, 0.5, 0.35));
        for d in decor.iter() {
            let s_it = buf.iter_at_mark(&d.start);
            let mut e_it = buf.iter_at_mark(&d.end);
            e_it.backward_char(); // the end mark sits at the start of the following line
            let (y0, y1, h1) = if d.kind == DecorKind::Code {
                // code panels include the padding lines above/below
                let (y0, _) = self.line_yrange(&s_it);
                let (y1, h1) = self.line_yrange(&e_it);
                (y0, y1, h1)
            } else {
                // quote bars hug the text, excluding paragraph spacing
                let first = self.iter_location(&s_it);
                let mut last_char = e_it;
                last_char.backward_char();
                let last = self.iter_location(&last_char);
                (first.y(), last.y(), last.height())
            };
            if y1 + h1 < visible.y() || y0 > visible.y() + visible.height() {
                continue;
            }
            let x = (PAGE_PAD + d.left) as f32;
            let height = (y1 + h1 - y0) as f32;
            let (rect, radius, color) = match d.kind {
                DecorKind::Code => (
                    graphene::Rect::new(x, y0 as f32, (width - PAGE_PAD) as f32 - x, height),
                    8.0,
                    code_bg,
                ),
                DecorKind::Quote(kind) => (
                    graphene::Rect::new(x + 2.0, y0 as f32, 3.0, height),
                    1.5,
                    kind.map(|k| alert_color(k, imp.dark.get())).unwrap_or(bar),
                ),
            };
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, radius));
            snapshot.append_color(&color, &rect);
            snapshot.pop();
        }
    }

    // --------------------------------------------------------------- links

    fn link_at(&self, x: f64, y: f64) -> Option<String> {
        let (bx, by) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let it = self.iter_at_location(bx, by)?;
        let links = self.imp().links.borrow();
        it.tags().iter().find_map(|t| links.get(t).cloned())
    }

    fn resolve(&self, src: &str) -> Option<gio::File> {
        if src.is_empty() {
            return None;
        }
        if let Some(scheme) = glib::Uri::peek_scheme(src) {
            if scheme == "data" {
                return None;
            }
            return Some(gio::File::for_uri(src));
        }
        let path = src.split(['#', '?']).next().unwrap_or("");
        let path = percent_decode(path);
        if path.starts_with('/') {
            return Some(gio::File::for_path(path));
        }
        self.imp().base_dir.borrow().as_ref().map(|d| d.resolve_relative_path(path))
    }
}

pub fn alert_color(kind: BlockQuoteKind, dark: bool) -> gdk::RGBA {
    let hex = match (kind, dark) {
        (BlockQuoteKind::Note, false) => "#0969da",
        (BlockQuoteKind::Note, true) => "#4493f8",
        (BlockQuoteKind::Tip, false) => "#1a7f37",
        (BlockQuoteKind::Tip, true) => "#3fb950",
        (BlockQuoteKind::Important, false) => "#8250df",
        (BlockQuoteKind::Important, true) => "#ab7df8",
        (BlockQuoteKind::Warning, false) => "#9a6700",
        (BlockQuoteKind::Warning, true) => "#d29922",
        (BlockQuoteKind::Caution, false) => "#cf222e",
        (BlockQuoteKind::Caution, true) => "#f85149",
    };
    gdk::RGBA::parse(hex).unwrap()
}

fn alert_title(kind: BlockQuoteKind) -> String {
    match kind {
        BlockQuoteKind::Note => format!("ⓘ  {}", tr("Note")),
        BlockQuoteKind::Tip => format!("💡  {}", tr("Tip")),
        BlockQuoteKind::Important => format!("❗  {}", tr("Important")),
        BlockQuoteKind::Warning => format!("⚠  {}", tr("Warning")),
        BlockQuoteKind::Caution => format!("⛔  {}", tr("Caution")),
    }
}

// =================================================================== helpers

/// Count words: runs of letters/digits, allowing inner apostrophes and hyphens.
pub fn count_words(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0;
    let mut in_word = false;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric() || c == '_' {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else if in_word && matches!(c, '\'' | '’' | '-')
            && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric())
        {
            // joiner inside a word: stay in the word
        } else {
            in_word = false;
        }
    }
    count
}

pub fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

pub fn percent_decode(s: &str) -> String {
    glib::Uri::unescape_string(s, None::<&str>).map(|g| g.to_string()).unwrap_or_else(|| s.to_owned())
}

/// Find all `<img …>` tags in an HTML fragment.
fn html_imgs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<img") {
        let start = pos + i;
        let end = lower[start..].find('>').map(|e| start + e + 1).unwrap_or(html.len());
        out.push(html[start..end].to_owned());
        pos = end;
    }
    out
}

/// Value of an attribute inside a single HTML tag.
fn html_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find(name) {
        let at = pos + i;
        pos = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[pos..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let (val, _) = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = &rest[1..];
                let end = body.find(q).unwrap_or(body.len());
                (&body[..end], ())
            }
            _ => {
                let end = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
                (&rest[..end], ())
            }
        };
        return Some(decode_entities(val));
    }
    None
}

fn strip_comments(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(e) => rest = &rest[i + e + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn is_br(tag: &str) -> bool {
    let t = tag.trim().to_ascii_lowercase();
    t.starts_with("<br") && t.ends_with('>')
}

/// Text content of an HTML fragment: tags removed, <br> as line breaks.
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find('>').map(|e| i + e + 1).unwrap_or(rest.len());
        if is_br(&rest[i..end]) || rest[i..end].to_ascii_lowercase().starts_with("</p") {
            out.push('\n');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn texture_from_bytes(bytes: &glib::Bytes) -> Option<gdk::Paintable> {
    if let Ok(t) = gdk::Texture::from_bytes(bytes) {
        return Some(t.upcast());
    }
    // SVG, GIF, WebP… go through gdk-pixbuf loaders
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write_bytes(bytes).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    #[allow(deprecated)]
    Some(gdk::Texture::for_pixbuf(&pixbuf).upcast())
}

// ================================================================= renderer

#[derive(Clone, Copy)]
enum Ctx {
    List { ordered: bool, next: u64, depth: usize },
    Quote,
}

#[derive(PartialEq, Clone, Copy)]
enum BlockKind {
    Paragraph,
    Implicit, // tight list items: text without a paragraph
    Heading,
}

struct Block {
    start: i32,
    marker: Option<String>,
    has_marker: bool,
    kind: BlockKind,
}

#[derive(Default)]
struct TableBuilder {
    aligns: Vec<Alignment>,
    rows: Vec<(Vec<String>, bool)>,
    row: Vec<String>,
    cell: String,
    in_head: bool,
}

struct Renderer<'a> {
    view: &'a DocumentView,
    buf: gtk::TextBuffer,
    table_: gtk::TextTagTable,
    dark: bool,
    ctx: Vec<Ctx>,
    marker: Option<String>,
    block: Option<Block>,
    inline: Vec<gtk::TextTag>,
    links: Vec<String>,
    heading: Option<(u8, String, Option<String>)>,
    heading_tag: Option<gtk::TextTag>,
    image: Option<(String, String)>,
    code: Option<(String, String)>,
    table: Option<TableBuilder>,
    html: Option<String>,
    quote_starts: Vec<(i32, Option<BlockQuoteKind>)>,
    footnote_starts: Vec<i32>,
    in_metadata: bool,
    had_footnotes: bool,
    slugs: HashMap<String, usize>,
    block_cache: HashMap<String, gtk::TextTag>,
    hl_cache: HashMap<(u8, u8, u8, bool, bool), gtk::TextTag>,
}

impl<'a> Renderer<'a> {
    fn new(view: &'a DocumentView, buf: &gtk::TextBuffer) -> Self {
        let mut r = Renderer {
            view,
            buf: buf.clone(),
            table_: buf.tag_table(),
            dark: adw::StyleManager::default().is_dark(),
            ctx: vec![],
            marker: None,
            block: None,
            inline: vec![],
            links: vec![],
            heading: None,
            heading_tag: None,
            image: None,
            code: None,
            table: None,
            html: None,
            quote_starts: vec![],
            footnote_starts: vec![],
            in_metadata: false,
            had_footnotes: false,
            slugs: HashMap::new(),
            block_cache: HashMap::new(),
            hl_cache: HashMap::new(),
        };
        r.setup_tags();
        r
    }

    fn setup_tags(&mut self) {
        let imp = self.view.imp();
        let sm = adw::StyleManager::default();
        let fg = self.view.color();
        let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.68);
        let accent = sm.accent_color().to_standalone_rgba(self.dark);
        let code_bg = gdk::RGBA::new(0.5, 0.5, 0.5, if self.dark { 0.13 } else { 0.09 });
        imp.code_bg.set(Some(code_bg));
        imp.quote_bar.set(Some(gdk::RGBA::new(0.5, 0.5, 0.5, 0.35)));
        imp.dark.set(self.dark);

        let add = |tag: gtk::TextTag| {
            self.table_.add(&tag);
        };
        add(gtk::TextTag::builder().name("body").line_height(1.25).build());
        for (lvl, scale) in [(1, 2.0), (2, 1.6), (3, 1.3), (4, 1.12), (5, 1.0), (6, 0.92)] {
            add(gtk::TextTag::builder()
                .name(format!("h{lvl}"))
                .weight(if lvl > 2 { 700 } else { 800 })
                .scale(scale)
                .line_height(1.1)
                .build());
        }
        add(gtk::TextTag::builder().name("h6-dim").foreground_rgba(&dim).build());
        add(gtk::TextTag::builder().name("bold").weight(700).build());
        add(gtk::TextTag::builder().name("italic").style(pango::Style::Italic).build());
        add(gtk::TextTag::builder().name("strike").strikethrough(true).build());
        add(gtk::TextTag::builder().name("code").family("monospace").background_rgba(&code_bg).build());
        add(gtk::TextTag::builder().name("codeblock").family("monospace").line_height(1.15).build());
        add(gtk::TextTag::builder().name("link").foreground_rgba(&accent).underline(pango::Underline::Single).build());
        add(gtk::TextTag::builder().name("quote").foreground_rgba(&dim).build());
        add(gtk::TextTag::builder().name("marker").foreground_rgba(&dim).weight(700).build());
        add(gtk::TextTag::builder().name("sup").rise(5 * pango::SCALE).scale(0.75).build());
        add(gtk::TextTag::builder().name("spacer").size_points(4.0).build());
        for kind in [BlockQuoteKind::Note, BlockQuoteKind::Tip, BlockQuoteKind::Important,
                     BlockQuoteKind::Warning, BlockQuoteKind::Caution] {
            add(gtk::TextTag::builder()
                .name(format!("alert-{kind:?}"))
                .foreground_rgba(&alert_color(kind, self.dark))
                .weight(700)
                .build());
        }
        add(gtk::TextTag::builder().name("match").background_rgba(&gdk::RGBA::new(0.96, 0.83, 0.18, 0.45)).build());
        add(gtk::TextTag::builder().name("match-current").background_rgba(&gdk::RGBA::new(1.0, 0.47, 0.0, 0.65)).build());
    }

    fn tag(&self, name: &str) -> gtk::TextTag {
        self.table_.lookup(name).unwrap_or_else(|| panic!("missing tag {name}"))
    }

    fn offset(&self) -> i32 {
        self.buf.char_count()
    }

    fn left(&self) -> i32 {
        self.ctx.iter().map(|c| match c {
            Ctx::List { .. } => LIST_INDENT,
            Ctx::Quote => QUOTE_INDENT,
        }).sum()
    }

    fn in_quote(&self) -> bool {
        self.ctx.iter().any(|c| matches!(c, Ctx::Quote))
    }

    fn char_tags(&self) -> Vec<gtk::TextTag> {
        let mut tags = vec![self.tag("body")];
        if self.in_quote() {
            tags.push(self.tag("quote"));
        }
        if let Some(h) = &self.heading_tag {
            tags.push(h.clone());
        }
        tags
    }

    /// Insert text with the contextual character tags plus `extra`.
    fn raw(&self, text: &str, extra: &[gtk::TextTag]) {
        if text.is_empty() {
            return;
        }
        let mut tags = self.char_tags();
        tags.extend_from_slice(extra);
        let refs: Vec<&gtk::TextTag> = tags.iter().collect();
        self.buf.insert_with_tags(&mut self.buf.end_iter(), text, &refs);
    }

    /// Insert inline content, opening an implicit block if needed.
    fn insert(&mut self, text: &str, extra: &[gtk::TextTag]) {
        if text.is_empty() {
            return;
        }
        self.ensure_block();
        let mut tags = self.inline.clone();
        tags.extend_from_slice(extra);
        self.raw(text, &tags);
    }

    fn ensure_block(&mut self) {
        if self.block.is_none() {
            self.begin_block(BlockKind::Implicit);
        }
        self.emit_block_marker();
    }

    fn emit_block_marker(&mut self) {
        let marker = self.block.as_mut().and_then(|b| b.marker.take());
        if let Some(m) = marker {
            self.raw(&m, &[self.tag("marker")]);
            self.raw("\t", &[]);
            if let Some(b) = self.block.as_mut() {
                b.has_marker = true;
            }
        }
    }

    fn begin_block(&mut self, kind: BlockKind) {
        self.block = Some(Block { start: self.offset(), marker: self.marker.take(), has_marker: false, kind });
    }

    fn end_block(&mut self, above: i32, below: i32) {
        self.emit_block_marker(); // empty list item
        let Some(b) = self.block.take() else { return };
        self.raw("\n", &[]);
        let tag = self.block_tag(self.left(), if b.has_marker { -LIST_INDENT } else { 0 }, 0, above, below, b.has_marker, false);
        self.buf.apply_tag(&tag, &self.buf.iter_at_offset(b.start), &self.buf.end_iter());
    }

    fn close_implicit(&mut self) {
        if self.block.as_ref().is_some_and(|b| b.kind == BlockKind::Implicit) {
            self.end_block(0, 4);
        }
    }

    /// A list item whose first child is a block (code, table, quote…) gets its marker on its own line.
    fn flush_marker_line(&mut self) {
        if self.marker.is_some() {
            self.begin_block(BlockKind::Paragraph);
            self.end_block(0, 2);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn block_tag(&mut self, left: i32, indent: i32, right: i32, above: i32, below: i32, tabs: bool, code: bool) -> gtk::TextTag {
        let key = format!("{left}:{indent}:{right}:{above}:{below}:{tabs}:{code}");
        if let Some(t) = self.block_cache.get(&key) {
            return t.clone();
        }
        let mut b = gtk::TextTag::builder()
            .left_margin(PAGE_PAD + left)
            .right_margin(PAGE_PAD + right)
            .indent(indent)
            .pixels_above_lines(above)
            .pixels_below_lines(below);
        if tabs {
            let mut arr = pango::TabArray::new(1, true);
            arr.set_tab(0, pango::TabAlign::Left, 0);
            b = b.tabs(&arr);
        }
        if code {
            b = b.wrap_mode(gtk::WrapMode::Char);
        }
        let tag = b.build();
        self.table_.add(&tag);
        self.block_cache.insert(key, tag.clone());
        tag
    }

    /// A paragraph-spacing override that must win over block tags.
    fn top_tag(&self, name: &str, above: i32, below: i32) -> gtk::TextTag {
        let tag = self.table_.lookup(name).unwrap_or_else(|| {
            let mut b = gtk::TextTag::builder().name(name);
            if above > 0 {
                b = b.pixels_above_lines(above);
            }
            if below > 0 {
                b = b.pixels_below_lines(below);
            }
            let t = b.build();
            self.table_.add(&t);
            t
        });
        tag.set_priority(self.table_.size() - 1);
        tag
    }

    fn space_after(&self, px: i32) {
        let end = self.buf.end_iter();
        let mut last = end;
        if !last.backward_line() {
            return;
        }
        self.buf.apply_tag(&self.top_tag(&format!("after-{px}"), 0, px), &last, &end);
    }

    fn decorate(&self, kind: DecorKind, start: i32, left: i32) {
        let s = self.buf.create_mark(None, &self.buf.iter_at_offset(start), true);
        let e = self.buf.create_mark(None, &self.buf.end_iter(), true);
        self.view.imp().decor.borrow_mut().push(Decor { kind, start: s, end: e, left });
    }

    fn anchor_widget(&self, widget: &impl IsA<gtk::Widget>) {
        let anchor = self.buf.create_child_anchor(&mut self.buf.end_iter());
        self.view.add_child_at_anchor(widget, &anchor);
        self.view.imp().wide.borrow_mut().push((widget.clone().upcast(), self.left()));
        let end = self.buf.end_iter();
        let mut before = end;
        before.backward_char();
        for t in self.char_tags() {
            self.buf.apply_tag(&t, &before, &end);
        }
    }

    fn unique_slug(&mut self, title: &str, custom: Option<String>) -> String {
        let base = custom.unwrap_or_else(|| slugify(title));
        let n = self.slugs.entry(base.clone()).or_insert(0);
        let slug = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
        *n += 1;
        slug
    }

    // ------------------------------------------------------------- events

    fn run(&mut self, source: &str) {
        let mut opts = Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_SMART_PUNCTUATION
            | Options::ENABLE_HEADING_ATTRIBUTES
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
        opts.insert(Options::ENABLE_GFM);
        for event in Parser::new_ext(source, opts) {
            self.event(event);
        }
        self.close_implicit();
    }

    fn event(&mut self, event: Event) {
        if self.in_metadata {
            if let Event::End(TagEnd::MetadataBlock(_)) = event {
                self.in_metadata = false;
            }
            return;
        }
        if let Some((_, code)) = self.code.as_mut() {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, code) = self.code.take().unwrap();
                    self.code_block(&lang, &code);
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
                    self.html_block(&html);
                }
                _ => {}
            }
            return;
        }
        if let Some((_, alt)) = self.image.as_mut() {
            match event {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = self.image.take().unwrap();
                    if let Some(tb) = self.table.as_mut() {
                        tb.cell.push_str(&glib::markup_escape_text(&alt));
                    } else {
                        self.ensure_block();
                        self.image(&src, &alt, None);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.table.is_some() && self.table_event(&event) {
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some((_, title, _)) = self.heading.as_mut() {
                    title.push_str(&t);
                }
                self.insert(&t, &[]);
            }
            Event::Code(t) => {
                if let Some((_, title, _)) = self.heading.as_mut() {
                    title.push_str(&t);
                }
                self.insert(&t, &[self.tag("code")]);
            }
            Event::SoftBreak => self.insert(" ", &[]),
            Event::HardBreak => self.insert("\u{2028}", &[]),
            Event::Rule => self.rule(),
            Event::TaskListMarker(done) => {
                let sym = if done { "☑" } else { "☐" }.to_owned();
                match self.block.as_mut() {
                    Some(b) if b.marker.is_some() => b.marker = Some(sym),
                    _ if self.marker.is_some() => self.marker = Some(sym),
                    _ => {}
                }
            }
            Event::InlineHtml(h) => {
                if is_br(&h) {
                    self.insert("\u{2028}", &[]);
                } else if h.trim_start().to_ascii_lowercase().starts_with("<img") {
                    self.ensure_block();
                    let width = html_attr(&h, "width").and_then(|w| w.parse().ok());
                    self.image(&html_attr(&h, "src").unwrap_or_default(), &html_attr(&h, "alt").unwrap_or_default(), width);
                }
            }
            Event::Html(h) => self.html_block(&h),
            Event::FootnoteReference(label) => {
                let url = format!("#fn-{}", slugify(&label));
                let anon = gtk::TextTag::new(None);
                self.table_.add(&anon);
                self.view.imp().links.borrow_mut().insert(anon.clone(), url);
                self.insert(&label, &[self.tag("sup"), self.tag("link"), anon]);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.close_implicit();
                self.begin_block(BlockKind::Paragraph);
            }
            Tag::Heading { level, id, .. } => {
                self.close_implicit();
                self.begin_block(BlockKind::Heading);
                let lvl = heading_level(level);
                self.heading_tag = Some(self.tag(&format!("h{lvl}")));
                self.heading = Some((lvl, String::new(), id.map(|s| s.to_string())));
            }
            Tag::BlockQuote(kind) => {
                self.close_implicit();
                self.flush_marker_line();
                self.ctx.push(Ctx::Quote);
                self.quote_starts.push((self.offset(), kind));
                if let Some(k) = kind {
                    self.begin_block(BlockKind::Paragraph);
                    self.raw(&alert_title(k), &[self.tag(&format!("alert-{k:?}"))]);
                    self.end_block(0, 4);
                }
            }
            Tag::CodeBlock(kind) => {
                self.close_implicit();
                self.flush_marker_line();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split([' ', ',', '{']).next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.close_implicit();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.close_implicit();
                self.flush_marker_line();
                let depth = self.ctx.iter().filter(|c| matches!(c, Ctx::List { .. })).count();
                self.ctx.push(Ctx::List { ordered: start.is_some(), next: start.unwrap_or(1), depth });
            }
            Tag::Item => {
                self.close_implicit();
                if let Some(Ctx::List { ordered, next, depth }) = self.ctx.last_mut() {
                    self.marker = Some(if *ordered {
                        let m = format!("{next}.");
                        *next += 1;
                        m
                    } else {
                        BULLETS[*depth % BULLETS.len()].to_owned()
                    });
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.close_implicit();
                if !self.had_footnotes {
                    self.had_footnotes = true;
                    self.rule();
                }
                let mark = self.buf.create_mark(None, &self.buf.end_iter(), true);
                self.view.imp().anchors.borrow_mut().push((format!("fn-{}", slugify(&label)), mark));
                let depth = self.ctx.iter().filter(|c| matches!(c, Ctx::List { .. })).count();
                self.ctx.push(Ctx::List { ordered: false, next: 0, depth });
                self.marker = Some(format!("{label}."));
                self.footnote_starts.push(self.offset());
            }
            Tag::Table(aligns) => {
                self.close_implicit();
                self.flush_marker_line();
                self.table = Some(TableBuilder { aligns, ..Default::default() });
            }
            Tag::Emphasis => self.inline.push(self.tag("italic")),
            Tag::Strong => self.inline.push(self.tag("bold")),
            Tag::Strikethrough => self.inline.push(self.tag("strike")),
            Tag::Link { dest_url, .. } => {
                let anon = gtk::TextTag::new(None);
                self.table_.add(&anon);
                self.view.imp().links.borrow_mut().insert(anon.clone(), dest_url.to_string());
                self.links.push(dest_url.to_string());
                self.inline.push(self.tag("link"));
                self.inline.push(anon);
            }
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
            Tag::MetadataBlock(_) => self.in_metadata = true,
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_block(0, 12),
            TagEnd::Heading(level) => {
                let lvl = heading_level(level);
                let start = self.block.as_ref().map(|b| b.start).unwrap_or(0);
                if lvl == 6 {
                    self.buf.apply_tag(&self.tag("h6-dim"), &self.buf.iter_at_offset(start), &self.buf.end_iter());
                }
                self.heading_tag = None;
                let (_, title, id) = self.heading.take().unwrap_or((lvl, String::new(), None));
                let title = title.trim().to_owned();
                let mark = self.buf.create_mark(None, &self.buf.iter_at_offset(start), true);
                let slug = self.unique_slug(&title, id);
                self.view.imp().headings.borrow_mut().push(Heading { level: lvl, title, slug, mark });
                let above = if start == 0 { 0 } else { match lvl { 1 => 12, 2 => 22, 3 => 18, _ => 14 } };
                self.end_block(above, if lvl <= 2 { 8 } else { 6 });
            }
            TagEnd::BlockQuote(_) => {
                self.close_implicit();
                self.ctx.pop();
                if let Some((start, kind)) = self.quote_starts.pop() {
                    self.decorate(DecorKind::Quote(kind), start, self.left());
                }
                self.space_after(14);
            }
            TagEnd::List(_) => {
                self.close_implicit();
                self.ctx.pop();
                if !self.ctx.iter().any(|c| matches!(c, Ctx::List { .. })) {
                    self.space_after(12);
                }
            }
            TagEnd::Item => {
                self.close_implicit();
                if self.marker.is_some() {
                    self.begin_block(BlockKind::Paragraph);
                    self.end_block(0, 4);
                }
            }
            TagEnd::FootnoteDefinition => {
                self.close_implicit();
                self.ctx.pop();
                self.footnote_starts.pop();
                self.space_after(6);
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.inline.pop();
            }
            TagEnd::Link => {
                self.inline.pop();
                self.inline.pop();
                self.links.pop();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------- blocks

    fn rule(&mut self) {
        self.close_implicit();
        self.begin_block(BlockKind::Paragraph);
        self.emit_block_marker();
        let sep = gtk::Separator::builder().css_classes(["md-rule"]).build();
        self.anchor_widget(&FitBox::new(&sep, true));
        self.end_block(10, 18);
    }

    fn code_block(&mut self, lang: &str, code: &str) {
        let code = code.trim_end_matches('\n');
        let start = self.offset();
        let mono = self.tag("codeblock");
        let syntax = (!lang.is_empty()).then(|| SYNTAXES.find_syntax_by_token(lang)).flatten();
        match syntax {
            Some(syntax) => {
                let theme = &THEMES.themes[if self.dark { "base16-ocean.dark" } else { "InspiredGitHub" }];
                let mut h = HighlightLines::new(syntax, theme);
                for line in LinesWithEndings::from(code) {
                    match h.highlight_line(line, &SYNTAXES) {
                        Ok(ranges) => {
                            for (style, text) in ranges {
                                let t = self.highlight_tag(style);
                                self.raw(text, &[mono.clone(), t]);
                            }
                        }
                        Err(_) => self.raw(line, &[mono.clone()]),
                    }
                }
            }
            None => self.raw(code, &[mono.clone()]),
        }
        self.raw("\n", &[]);
        let left = self.left();
        let block = self.block_tag(left + CODE_PAD, 0, CODE_PAD + 4, 0, 0, false, true);
        let s_it = self.buf.iter_at_offset(start);
        self.buf.apply_tag(&block, &s_it, &self.buf.end_iter());
        self.decorate(DecorKind::Code, start, left);
        // inner padding
        let mut first_end = s_it;
        first_end.forward_to_line_end();
        self.buf.apply_tag(&self.top_tag("code-first", 12, 0), &s_it, &first_end);
        let mut last = self.buf.end_iter();
        last.backward_char();
        last.set_line_offset(0);
        self.buf.apply_tag(&self.top_tag("code-last", 0, 12), &last, &self.buf.end_iter());
        self.raw("\n", &[self.tag("spacer")]);
    }

    fn highlight_tag(&mut self, style: syntect::highlighting::Style) -> gtk::TextTag {
        let c = style.foreground;
        let bold = style.font_style.contains(FontStyle::BOLD);
        let italic = style.font_style.contains(FontStyle::ITALIC);
        let key = (c.r, c.g, c.b, bold, italic);
        if let Some(t) = self.hl_cache.get(&key) {
            return t.clone();
        }
        let mut b = gtk::TextTag::builder()
            .foreground_rgba(&gdk::RGBA::new(c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, 1.0));
        if bold {
            b = b.weight(700);
        }
        if italic {
            b = b.style(pango::Style::Italic);
        }
        let tag = b.build();
        self.table_.add(&tag);
        self.hl_cache.insert(key, tag.clone());
        tag
    }

    fn html_block(&mut self, html: &str) {
        let html = strip_comments(html);
        let imgs = html_imgs(&html);
        let text = html_text(&html);
        let text = text.trim();
        if imgs.is_empty() && text.is_empty() {
            return;
        }
        self.close_implicit();
        self.begin_block(BlockKind::Paragraph);
        self.emit_block_marker();
        for img in &imgs {
            let width = html_attr(img, "width").and_then(|w| w.trim_end_matches("px").parse().ok());
            self.image(&html_attr(img, "src").unwrap_or_default(), &html_attr(img, "alt").unwrap_or_default(), width);
            self.raw(" ", &[]);
        }
        if !text.is_empty() {
            if !imgs.is_empty() {
                self.raw("\u{2028}", &[]);
            }
            let joined: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            self.raw(&joined.join("\u{2028}"), &[]);
        }
        self.end_block(0, 12);
    }

    // ------------------------------------------------------------- images

    fn image(&mut self, src: &str, alt: &str, max_width: Option<i32>) {
        let link = self.links.last().cloned();
        let slot = gtk::Box::builder().valign(gtk::Align::End).build();
        let img = FitImage::new(max_width);
        if !alt.is_empty() {
            img.update_property(&[gtk::accessible::Property::Label(alt)]);
        }
        if let Some(url) = link {
            img.set_cursor_from_name(Some("pointer"));
            img.set_tooltip_text(Some(&url));
            let click = gtk::GestureClick::new();
            let view = self.view.downgrade();
            click.connect_released(move |_, _, _, _| {
                if let Some(v) = view.upgrade() {
                    v.activate_link(&url);
                }
            });
            img.add_controller(click);
        }
        slot.append(&img);
        self.anchor_widget(&slot);

        let fallback_text = format!("🖼 {}", if alt.is_empty() { src } else { alt });
        let fallback = glib::clone!(
            #[weak]
            slot,
            #[weak]
            img,
            move || {
                if img.parent().is_some() {
                    slot.remove(&img);
                    let label = gtk::Label::builder().label(&fallback_text).css_classes(["dim-label", "md-img-alt"]).build();
                    slot.append(&label);
                }
            }
        );
        let Some(file) = self.view.resolve(src) else {
            fallback();
            return;
        };
        let view = self.view.downgrade();
        glib::spawn_future_local(async move {
            match file.load_bytes_future().await {
                Ok((bytes, _)) => match texture_from_bytes(&bytes) {
                    Some(p) => {
                        img.set_paintable(p);
                        if let Some(v) = view.upgrade() {
                            v.imp().last_width.set(-1);
                            v.queue_allocate();
                        }
                    }
                    None => fallback(),
                },
                Err(_) => fallback(),
            }
        });
    }

    // ------------------------------------------------------------- tables

    /// Route events while inside a table. Returns true when consumed.
    fn table_event(&mut self, event: &Event) -> bool {
        let tb = self.table.as_mut().unwrap();
        let esc = |s: &str| glib::markup_escape_text(s).to_string();
        match event {
            Event::Start(Tag::TableHead) => {
                tb.in_head = true;
                tb.row.clear();
            }
            Event::End(TagEnd::TableHead) => {
                tb.rows.push((std::mem::take(&mut tb.row), true));
                tb.in_head = false;
            }
            Event::Start(Tag::TableRow) => tb.row.clear(),
            Event::End(TagEnd::TableRow) => tb.rows.push((std::mem::take(&mut tb.row), false)),
            Event::Start(Tag::TableCell) => tb.cell.clear(),
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut tb.cell);
                tb.row.push(cell);
            }
            Event::Text(t) => tb.cell.push_str(&esc(t)),
            Event::Code(t) => tb.cell.push_str(&format!("<tt>{}</tt>", esc(t))),
            Event::SoftBreak => tb.cell.push(' '),
            Event::HardBreak => tb.cell.push('\n'),
            Event::InlineHtml(h) if is_br(h) => tb.cell.push('\n'),
            Event::Start(Tag::Strong) => tb.cell.push_str("<b>"),
            Event::End(TagEnd::Strong) => tb.cell.push_str("</b>"),
            Event::Start(Tag::Emphasis) => tb.cell.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => tb.cell.push_str("</i>"),
            Event::Start(Tag::Strikethrough) => tb.cell.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => tb.cell.push_str("</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => tb.cell.push_str(&format!("<a href=\"{}\">", esc(dest_url))),
            Event::End(TagEnd::Link) => tb.cell.push_str("</a>"),
            Event::Start(Tag::Image { dest_url, .. }) => self.image = Some((dest_url.to_string(), String::new())),
            Event::End(TagEnd::Table) => {
                let tb = self.table.take().unwrap();
                self.build_table(tb);
            }
            _ => return matches!(event, Event::Start(_) | Event::End(_) | Event::Html(_) | Event::InlineHtml(_)),
        }
        true
    }

    fn build_table(&mut self, tb: TableBuilder) {
        let grid = gtk::Grid::builder().css_classes(["md-table"]).build();
        let n_rows = tb.rows.len();
        let mut body_index = 0;
        for (r, (cells, header)) in tb.rows.iter().enumerate() {
            if !header {
                body_index += 1;
            }
            for (c, markup) in cells.iter().enumerate() {
                let xalign = match tb.aligns.get(c) {
                    Some(Alignment::Center) => 0.5,
                    Some(Alignment::Right) => 1.0,
                    _ => 0.0,
                };
                let label = gtk::Label::builder()
                    .use_markup(true)
                    .wrap(true)
                    .wrap_mode(pango::WrapMode::WordChar)
                    .max_width_chars(48)
                    .selectable(true)
                    .xalign(xalign)
                    .hexpand(true)
                    .css_classes(["md-cell"])
                    .build();
                label.set_markup(markup); // content is escaped; <a href> is GTK markup
                if *header {
                    label.add_css_class("md-th");
                } else if body_index % 2 == 0 {
                    label.add_css_class("md-striped");
                }
                if r == n_rows - 1 {
                    label.add_css_class("md-last-row");
                }
                let view = self.view.downgrade();
                label.connect_activate_link(move |_, uri| {
                    if let Some(v) = view.upgrade() {
                        v.activate_link(uri);
                    }
                    glib::Propagation::Stop
                });
                grid.attach(&label, c as i32, r as i32, 1, 1);
            }
        }
        let frame = gtk::Frame::builder().child(&grid).css_classes(["md-table-frame"]).build();
        self.begin_block(BlockKind::Paragraph);
        self.emit_block_marker();
        self.anchor_widget(&FitBox::new(&frame, false));
        self.end_block(2, 16);
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(count_words("Hello, world! It's a well-known fact — 42 times."), 8);
        assert_eq!(count_words("  "), 0);
        assert_eq!(count_words("año ñandú"), 2);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("Instalación rápida"), "instalación-rápida");
    }

    #[test]
    fn html_helpers() {
        let tag = r#"<img src="a.png" alt='An "x"' width=200>"#;
        assert_eq!(html_attr(tag, "src").as_deref(), Some("a.png"));
        assert_eq!(html_attr(tag, "width").as_deref(), Some("200"));
        assert_eq!(html_imgs("<p><img src=1><IMG src=2></p>").len(), 2);
        assert_eq!(html_text("<p>Hi&amp;<br/>there</p>").trim(), "Hi&\nthere");
        assert_eq!(strip_comments("a<!-- x -->b"), "ab");
    }
}
