//! The main window: header bar, split view, search, history, live reload, status bar.

use crate::config::{self, State, APP_ID, MIME_TYPES};
use crate::document::Heading;
use crate::editor::rich::RichView;
use crate::editor::split_scroll::{Side, SplitScroll};
use crate::editor::source_view::SourceView;
use crate::editor::{self, EditorMode, Insert, Toggle, STYLE_ACTION};
use crate::markdown::format::FormatCommand;
use crate::i18n::{fill, group_digits, tr, trn};
use crate::sidebar::{is_markdown_name, FilesPanel, OutlinePanel, MARKDOWN_SUFFIXES};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const README_NAMES: &[&str] = &["README.md", "readme.md", "Readme.md", "README.markdown", "index.md", "INDEX.md"];
const WORDS_PER_MINUTE: f64 = 220.0;

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
}

fn display_path(f: &gio::File) -> String {
    let Some(path) = f.path() else { return f.parse_name().to_string() };
    let path = path.to_string_lossy().into_owned();
    let home = glib::home_dir().to_string_lossy().into_owned();
    match path.strip_prefix(&home) {
        Some(rest) => format!("~{rest}"),
        None => path,
    }
}

fn basename(f: &gio::File) -> String {
    f.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_default()
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    split: adw::OverlaySplitView,
    side_toggles: adw::ToggleGroup,
    outline: Rc<OutlinePanel>,
    files: Rc<FilesPanel>,
    title: adw::WindowTitle,
    nav_box: gtk::Box,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    match_label: gtk::Label,
    /// The formatted editor.
    doc: RichView,
    scroller: gtk::ScrolledWindow,
    source: SourceView,
    source_scroller: gtk::ScrolledWindow,
    paned: gtk::Paned,
    format_row: gtk::Box,
    mode_groups: [adw::ToggleGroup; 2],
    content_stack: gtk::Stack,
    error_page: adw::StatusPage,
    banner: adw::Banner,
    toasts: adw::ToastOverlay,
    status_bar: gtk::Box,
    words_label: gtk::Label,
    reading_label: gtk::Label,
    actions: gio::SimpleActionGroup,

    current: RefCell<Option<gio::File>>,
    folder: RefCell<Option<gio::File>>,
    history: RefCell<Vec<(gio::File, f64)>>,
    forward: RefCell<Vec<(gio::File, f64)>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_source: RefCell<Option<glib::SourceId>>,
    stats_source: RefCell<Option<glib::SourceId>>,
    matches: RefCell<Vec<(i32, i32)>>,
    match_index: Cell<i32>,
    total_words: Cell<u64>,
    style_handlers: RefCell<Vec<glib::SignalHandlerId>>,

    mode: Cell<EditorMode>,
    /// The file's bytes as last read or written; `None` when nothing editable is open.
    disk: RefCell<Option<Vec<u8>>>,
    /// Edits not written to the file yet.
    dirty: Cell<bool>,
    save_source: RefCell<Option<glib::SourceId>>,
    render_source: RefCell<Option<glib::SourceId>>,
    /// Edits in the formatted editor waiting to reach the Markdown text.
    rich_sync: RefCell<Option<glib::SourceId>>,
    outline_source: RefCell<Option<glib::SourceId>>,
    /// Which editor the tools act on in the split view: the last one with focus.
    last_editor: Cell<EditorMode>,
    split_scroll: Rc<SplitScroll>,
    /// Where each heading of the Contents panel starts in the source, in characters.
    heading_offsets: RefCell<Vec<i32>>,
    conflict_open: Cell<bool>,
}

impl Window {
    pub fn all() -> Vec<Rc<Window>> {
        WINDOWS.with(|w| w.borrow().clone())
    }

    pub fn for_widget(win: &gtk::Window) -> Option<Rc<Window>> {
        WINDOWS.with(|w| w.borrow().iter().find(|x| x.window.upcast_ref::<gtk::Window>() == win).cloned())
    }

    pub fn new(app: &adw::Application) -> Rc<Self> {
        let st = State::load();
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Amoxtli")
            .default_width(st.width)
            .default_height(st.height)
            .build();
        if st.maximized {
            window.maximize();
        }

        // ---- sidebar
        let outline = OutlinePanel::new();
        let files = FilesPanel::new();
        let side_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        side_stack.add_named(&outline.widget, Some("contents"));
        side_stack.add_named(&files.widget, Some("files"));
        let side_toggles = adw::ToggleGroup::builder().css_classes(["flat", "round"]).homogeneous(true).build();
        side_toggles.add(adw::Toggle::builder().name("contents").label(tr("Contents")).tooltip(tr("Table of Contents")).build());
        side_toggles.add(adw::Toggle::builder().name("files").label(tr("Files")).tooltip(tr("Folder")).build());
        side_toggles.bind_property("active-name", &side_stack, "visible-child-name").bidirectional().sync_create().build();
        let side_header = adw::HeaderBar::builder().show_end_title_buttons(false).title_widget(&side_toggles).build();
        let sidebar = adw::ToolbarView::builder().content(&side_stack).build();
        sidebar.add_top_bar(&side_header);

        // ---- header
        let header = adw::HeaderBar::new();
        let sidebar_button = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(format!("{} (F9)", tr("Toggle Sidebar")))
            .build();
        header.pack_start(&sidebar_button);
        let nav_box = gtk::Box::builder().css_classes(["linked"]).build();
        nav_box.append(&gtk::Button::builder().icon_name("go-previous-symbolic").action_name("win.back")
            .tooltip_text(format!("{} (Alt+←)", tr("Back"))).build());
        nav_box.append(&gtk::Button::builder().icon_name("go-next-symbolic").action_name("win.forward")
            .tooltip_text(format!("{} (Alt+→)", tr("Forward"))).build());
        header.pack_start(&nav_box);
        let open_menu = gio::Menu::new();
        open_menu.append(Some(&tr("Open Folder…")), Some("win.open-folder"));
        let open_btn = adw::SplitButton::builder()
            .label(tr("Open"))
            .action_name("win.open")
            .menu_model(&open_menu)
            .tooltip_text(format!("{} (Ctrl+O)", tr("Open File")))
            .dropdown_tooltip(tr("More Open Options"))
            .build();
        header.pack_start(&open_btn);
        // narrow windows: same button with an icon instead of the label (swapped by the breakpoint)
        let open_icon_btn = adw::SplitButton::builder()
            .icon_name("document-open-symbolic")
            .action_name("win.open")
            .menu_model(&open_menu)
            .tooltip_text(format!("{} (Ctrl+O)", tr("Open File")))
            .dropdown_tooltip(tr("More Open Options"))
            .visible(false)
            .build();
        header.pack_start(&open_icon_btn);
        // Next to Open: switching to the source is how editing starts.
        let mode_groups = [mode_toggles(false), mode_toggles(true)];
        mode_groups[1].set_visible(false);
        header.pack_start(&mode_groups[0]);
        header.pack_start(&mode_groups[1]);
        let title = adw::WindowTitle::new("Amoxtli", "");
        header.set_title_widget(Some(&title));
        header.pack_end(&gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu())
            .primary(true)
            .tooltip_text(tr("Main Menu"))
            .build());
        let search_button = gtk::ToggleButton::builder()
            .icon_name("edit-find-symbolic")
            .tooltip_text(format!("{} (Ctrl+F)", tr("Find")))
            .build();
        header.pack_end(&search_button);
        let typography_button = gtk::MenuButton::builder()
            .icon_name("font-select-symbolic")
            .menu_model(&crate::application::typography_menu())
            .tooltip_text(tr("Font and Text Size"))
            .build();
        header.pack_end(&typography_button);

        // ---- search bar
        let search_entry = gtk::SearchEntry::builder().placeholder_text(tr("Find in document")).hexpand(true).build();
        let match_label = gtk::Label::builder().css_classes(["dim-label", "numeric"]).width_chars(8).build();
        let prev_b = gtk::Button::builder().icon_name("go-up-symbolic")
            .tooltip_text(format!("{} (Shift+Enter)", tr("Previous Match"))).build();
        let next_b = gtk::Button::builder().icon_name("go-down-symbolic")
            .tooltip_text(format!("{} (Enter)", tr("Next Match"))).build();
        let sbox = gtk::Box::builder().spacing(6).build();
        sbox.append(&search_entry);
        sbox.append(&match_label);
        let linked = gtk::Box::builder().css_classes(["linked"]).build();
        linked.append(&prev_b);
        linked.append(&next_b);
        sbox.append(&linked);
        let search_bar = gtk::SearchBar::builder().child(&adw::Clamp::builder().child(&sbox).maximum_size(560).build()).build();
        search_bar.connect_entry(&search_entry);
        search_bar.set_key_capture_widget(Some(&window));
        search_bar.bind_property("search-mode-enabled", &search_button, "active").bidirectional().sync_create().build();

        // ---- document
        let doc = RichView::new();
        doc.set_extra_menu(Some(&editor::context_menu()));
        let clamp = adw::ClampScrollable::builder().child(&doc).maximum_size(920).tightening_threshold(640).build();
        let scroller = gtk::ScrolledWindow::builder().child(&clamp).vexpand(true).hexpand(true).build();
        let source = SourceView::new();
        source.set_extra_menu(Some(&editor::context_menu()));
        let source_clamp = adw::ClampScrollable::builder().child(&source).maximum_size(920).tightening_threshold(640).build();
        let source_scroller = gtk::ScrolledWindow::builder().child(&source_clamp).vexpand(true).hexpand(true).build();
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&source_scroller)
            .end_child(&scroller)
            .wide_handle(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name(APP_ID)
            .title("Amoxtli")
            .description(format!("{}\n{}", tr("Open a Markdown file or a folder of notes."), tr("You can also drop files here.")))
            .build();
        let ebox = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
        ebox.append(&gtk::Button::builder().label(tr("Open File…")).action_name("win.open")
            .css_classes(["pill", "suggested-action"]).build());
        ebox.append(&gtk::Button::builder().label(tr("Open Folder…")).action_name("win.open-folder")
            .css_classes(["pill"]).build());
        empty.set_child(Some(&ebox));
        let pick_page = adw::StatusPage::builder()
            .icon_name("folder-documents-symbolic")
            .title(tr("Choose a Document"))
            .description(tr("Select a Markdown file from the sidebar"))
            .build();
        let error_page = adw::StatusPage::builder().icon_name("dialog-error-symbolic").title(tr("Unable to Open File")).build();
        let content_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        content_stack.add_named(&empty, Some("empty"));
        content_stack.add_named(&paned, Some("doc"));
        content_stack.add_named(&pick_page, Some("pick"));
        content_stack.add_named(&error_page, Some("error"));

        let banner = adw::Banner::builder()
            .title(tr("Amoxtli is not your default Markdown viewer"))
            .button_label(tr("Make Default"))
            .action_name("app.make-default")
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content_stack));
        let content = adw::ToolbarView::builder().content(&toasts).top_bar_style(adw::ToolbarStyle::RaisedBorder).build();
        content.add_top_bar(&header);
        let format_bar = editor::format_bar();
        format_bar.set_hexpand(true);
        let format_row = gtk::Box::builder().css_classes(["toolbar"]).visible(false).build();
        format_row.append(&format_bar);
        content.add_top_bar(&format_row);
        content.add_top_bar(&search_bar);
        content.add_top_bar(&banner);

        // ---- status bar: word count and reading time
        let words_label = gtk::Label::builder().css_classes(["dim-label", "caption", "numeric"]).build();
        let reading_label = gtk::Label::builder().css_classes(["dim-label", "caption", "numeric"]).build();
        let status_bar = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::End)
            .margin_start(12)
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .visible(false)
            .build();
        status_bar.append(&words_label);
        status_bar.append(&gtk::Label::builder().label("·").css_classes(["dim-label", "caption"]).build());
        status_bar.append(&reading_label);
        content.add_bottom_bar(&status_bar);

        // ---- split view
        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(220.0)
            .max_sidebar_width(340.0)
            .sidebar_width_fraction(0.26)
            .show_sidebar(st.sidebar)
            .build();
        split.bind_property("show-sidebar", &sidebar_button, "active").bidirectional().sync_create().build();
        window.set_content(Some(&split));
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 680sp").unwrap());
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        bp.add_setter(&open_btn, "visible", Some(&false.to_value()));
        bp.add_setter(&open_icon_btn, "visible", Some(&true.to_value()));
        bp.add_setter(&mode_groups[0], "visible", Some(&false.to_value()));
        bp.add_setter(&mode_groups[1], "visible", Some(&true.to_value()));
        bp.add_setter(&search_button, "visible", Some(&false.to_value())); // still in the menu, Ctrl+F, or just type
        bp.add_setter(&typography_button, "visible", Some(&false.to_value())); // in the main menu too
        window.add_breakpoint(bp);

        let split_scroll = SplitScroll::new(&source, &doc, &source_scroller, &scroller);
        let this = Rc::new(Self {
            window,
            split,
            side_toggles,
            outline,
            files,
            title,
            nav_box,
            search_bar,
            search_entry,
            match_label,
            doc,
            scroller,
            source,
            source_scroller,
            paned,
            format_row,
            mode_groups,
            content_stack,
            error_page,
            banner,
            toasts,
            status_bar,
            words_label,
            reading_label,
            actions: gio::SimpleActionGroup::new(),
            current: RefCell::new(None),
            folder: RefCell::new(None),
            history: RefCell::new(vec![]),
            forward: RefCell::new(vec![]),
            monitor: RefCell::new(None),
            reload_source: RefCell::new(None),
            stats_source: RefCell::new(None),
            matches: RefCell::new(vec![]),
            match_index: Cell::new(-1),
            total_words: Cell::new(0),
            style_handlers: RefCell::new(vec![]),
            mode: Cell::new(EditorMode::from_name(&st.mode).unwrap_or(EditorMode::Format)),
            disk: RefCell::new(None),
            dirty: Cell::new(false),
            save_source: RefCell::new(None),
            render_source: RefCell::new(None),
            rich_sync: RefCell::new(None),
            outline_source: RefCell::new(None),
            last_editor: Cell::new(EditorMode::Source),
            split_scroll,
            heading_offsets: RefCell::new(vec![]),
            conflict_open: Cell::new(false),
        });
        WINDOWS.with(|w| w.borrow_mut().push(this.clone()));
        this.connect_signals(&prev_b, &next_b);
        this.setup_actions();
        this.update_nav();
        this.refresh_banner();
        this.set_mode(this.mode.get());
        this
    }

    fn connect_signals(self: &Rc<Self>, prev_b: &gtk::Button, next_b: &gtk::Button) {
        let weak = Rc::downgrade(self);
        macro_rules! with {
            ($body:expr) => {{
                let weak = weak.clone();
                move |_: &_| {
                    if let Some(w) = weak.upgrade() {
                        #[allow(clippy::redundant_closure_call)]
                        ($body)(&w);
                    }
                }
            }};
        }

        self.outline.connect_heading_activated({
            let weak = weak.clone();
            move |h| {
                if let Some(w) = weak.upgrade() {
                    w.scroll_to_heading(&h);
                }
            }
        });
        self.files.connect_file_activated({
            let weak = weak.clone();
            move |f| {
                if let Some(w) = weak.upgrade() {
                    w.open_file(&f, true, None, false, 0.0);
                }
            }
        });
        self.doc.connect_link_activated({
            let weak = weak.clone();
            move |url| {
                if let Some(w) = weak.upgrade() {
                    w.on_link(url);
                }
            }
        });
        self.scroller.vadjustment().connect_value_changed(with!(|w: &Rc<Window>| w.on_scrolled()));
        self.source_scroller.vadjustment().connect_value_changed(with!(|w: &Rc<Window>| w.on_source_scrolled()));
        self.source.connect_edited({
            let weak = weak.clone();
            move || {
                if let Some(w) = weak.upgrade() {
                    w.on_edited();
                }
            }
        });
        self.doc.connect_edited({
            let weak = weak.clone();
            move || {
                if let Some(w) = weak.upgrade() {
                    w.on_rich_edited();
                }
            }
        });
        self.doc.connect_formats_changed({
            let weak = weak.clone();
            move || {
                if let Some(w) = weak.upgrade() {
                    if w.doc.has_focus() || w.doc.focused_cell().is_some() {
                        w.last_editor.set(EditorMode::Format);
                    }
                    w.update_format_state();
                    w.queue_stats();
                }
            }
        });
        // Before taking edits, each editor catches up with the other one's.
        self.doc.connect_has_focus_notify({
            let weak = weak.clone();
            move |doc| {
                if let (Some(w), true) = (weak.upgrade(), doc.has_focus()) {
                    w.catch_up(Side::Formatted);
                }
            }
        });
        self.source.connect_has_focus_notify({
            let weak = weak.clone();
            move |source| {
                if let (Some(w), true) = (weak.upgrade(), source.has_focus()) {
                    w.catch_up(Side::Source);
                }
            }
        });
        self.source.connect_formats_changed({
            let weak = weak.clone();
            move || {
                if let Some(w) = weak.upgrade() {
                    w.update_format_state();
                }
            }
        });
        self.source.buffer().connect_mark_set({
            let weak = weak.clone();
            move |buf, _, mark| {
                if mark == &buf.get_insert() || mark == &buf.selection_bound() {
                    if let Some(w) = weak.upgrade() {
                        w.queue_stats();
                    }
                }
            }
        });
        for group in &self.mode_groups {
            group.connect_active_name_notify({
                let weak = weak.clone();
                move |g| {
                    let (Some(w), Some(name)) = (weak.upgrade(), g.active_name()) else { return };
                    match EditorMode::from_name(&name) {
                        Some(mode) if mode != w.mode.get() => w.set_mode(mode),
                        _ => {}
                    }
                }
            });
        }

        self.search_entry.connect_search_changed(with!(|w: &Rc<Window>| w.on_search_changed()));
        self.search_entry.connect_activate(with!(|w: &Rc<Window>| w.step_match(1)));
        self.search_entry.connect_next_match(with!(|w: &Rc<Window>| w.step_match(1)));
        self.search_entry.connect_previous_match(with!(|w: &Rc<Window>| w.step_match(-1)));
        self.search_entry.connect_stop_search(with!(|w: &Rc<Window>| w.search_bar.set_search_mode(false)));
        prev_b.connect_clicked(with!(|w: &Rc<Window>| w.step_match(-1)));
        next_b.connect_clicked(with!(|w: &Rc<Window>| w.step_match(1)));
        self.search_bar.connect_search_mode_enabled_notify(with!(|w: &Rc<Window>| {
            if !w.search_bar.is_search_mode() {
                w.clear_matches();
                w.match_label.set_label("");
            }
        }));

        // drag and drop
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop({
            let weak = weak.clone();
            move |_, value, _, _| {
                let (Some(w), Ok(list)) = (weak.upgrade(), value.get::<gdk::FileList>()) else { return false };
                let files = list.files();
                let Some(first) = files.first() else { return false };
                w.open_any(first);
                if let Some(app) = w.window.application().and_downcast::<adw::Application>() {
                    for f in &files[1..] {
                        let nw = Window::new(&app);
                        nw.open_any(f);
                        nw.window.present();
                    }
                }
                true
            }
        });
        self.window.add_controller(drop);

        let sm = adw::StyleManager::default();
        let restyle = {
            let weak = weak.clone();
            move |_: &adw::StyleManager| {
                if let Some(w) = weak.upgrade() {
                    w.restyle();
                }
            }
        };
        self.style_handlers.replace(vec![
            sm.connect_dark_notify(restyle.clone()),
            sm.connect_high_contrast_notify(restyle.clone()),
            sm.connect_accent_color_notify(restyle),
        ]);

        self.window.connect_close_request({
            let weak = weak.clone();
            move |_| {
                if let Some(w) = weak.upgrade() {
                    if !w.can_close() {
                        return glib::Propagation::Stop;
                    }
                    w.on_close();
                }
                glib::Propagation::Proceed
            }
        });
    }

    fn setup_actions(self: &Rc<Self>) {
        let add = |name: &str, f: fn(&Rc<Window>)| -> gio::SimpleAction {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            self.actions.add_action(&action);
            action
        };
        add("open", |w| w.choose_file());
        add("open-folder", |w| w.choose_folder());
        add("reload", |w| w.reload());
        add("find", |w| {
            if w.current.borrow().is_some() {
                w.search_bar.set_search_mode(true);
            }
        });
        add("toggle-sidebar", |w| w.split.set_show_sidebar(!w.split.shows_sidebar()));
        add("show-contents", |w| w.show_side("contents"));
        add("show-files", |w| w.show_side("files"));
        add("back", |w| w.go_back());
        add("forward", |w| w.go_forward());
        add("edit", |w| w.edit_externally());
        add("show-in-files", |w| w.show_in_files());
        add("copy-path", |w| w.copy_path());
        add("close", |w| w.window.close());
        add("top", |w| {
            if w.source.has_focus() {
                w.source.reveal(0);
            } else {
                w.scroller.vadjustment().set_value(0.0);
            }
        });
        add("save", |w| {
            w.flush_save();
        });
        add(Insert::Table.action(), |w| w.insert_table());
        add(Insert::HorizontalRule.action(), |w| w.apply_format(&FormatCommand::HorizontalRule));

        let weak = Rc::downgrade(self);
        let mode = gio::SimpleAction::new_stateful("mode", Some(glib::VariantTy::STRING), &self.mode.get().name().to_variant());
        mode.connect_activate(move |_, param| {
            let (Some(w), Some(mode)) = (weak.upgrade(), param.and_then(|p| p.str()).and_then(EditorMode::from_name)) else { return };
            w.set_mode(mode);
        });
        self.actions.add_action(&mode);

        for tool in Toggle::ALL {
            let action = gio::SimpleAction::new_stateful(tool.action(), None, &false.to_variant());
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    w.run_tool(tool);
                }
            });
            self.actions.add_action(&action);
        }
        let weak = Rc::downgrade(self);
        let style = gio::SimpleAction::new_stateful(STYLE_ACTION, Some(glib::VariantTy::STRING), &"body".to_variant());
        style.connect_activate(move |action, param| {
            let (Some(w), Some(name)) = (weak.upgrade(), param.and_then(|p| p.str())) else { return };
            // Choosing the current style keeps it, as in a word processor.
            if action.state().and_then(|s| s.str().map(str::to_owned)).as_deref() != Some(name) {
                w.apply_format(&FormatCommand::Heading(editor::style_level(name)));
            }
        });
        self.actions.add_action(&style);
        self.window.insert_action_group("win", Some(&self.actions));
        self.update_doc_actions();
    }

    fn action(&self, name: &str) -> gio::SimpleAction {
        self.actions.lookup_action(name).and_downcast::<gio::SimpleAction>().unwrap()
    }

    fn update_doc_actions(&self) {
        let has = self.current.borrow().is_some();
        for name in ["reload", "edit", "show-in-files", "copy-path"] {
            self.action(name).set_enabled(has);
        }
        self.update_editing();
    }

    // ============================================================= opening

    pub fn is_empty(&self) -> bool {
        self.current.borrow().is_none() && self.folder.borrow().is_none()
    }

    fn choose_file(self: &Rc<Self>) {
        let md = gtk::FileFilter::new();
        md.set_name(Some(&tr("Markdown Documents")));
        for m in MIME_TYPES {
            md.add_mime_type(m);
        }
        for s in MARKDOWN_SUFFIXES {
            md.add_suffix(&s[1..]);
        }
        let txt = gtk::FileFilter::new();
        txt.set_name(Some(&tr("Text Files")));
        txt.add_mime_type("text/plain");
        let any = gtk::FileFilter::new();
        any.set_name(Some(&tr("All Files")));
        any.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&md);
        filters.append(&txt);
        filters.append(&any);
        let dialog = gtk::FileDialog::builder()
            .title(tr("Open Markdown File"))
            .modal(true)
            .filters(&filters)
            .default_filter(&md)
            .build();
        let start = self.current.borrow().as_ref().and_then(|f| f.parent()).or_else(|| self.folder.borrow().clone());
        if let Some(start) = start {
            dialog.set_initial_folder(Some(&start));
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(f) = dialog.open_future(Some(&this.window)).await {
                this.open_file(&f, true, None, false, 0.0);
            }
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title(tr("Open Folder")).modal(true).build();
        let start = self.folder.borrow().clone().or_else(|| self.current.borrow().as_ref().and_then(|f| f.parent()));
        if let Some(start) = start {
            dialog.set_initial_folder(Some(&start));
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(f) = dialog.select_folder_future(Some(&this.window)).await {
                this.open_folder(&f);
            }
        });
    }

    pub fn open_any(self: &Rc<Self>, f: &gio::File) {
        if f.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE) == gio::FileType::Directory {
            self.open_folder(f);
        } else {
            self.open_file(f, true, None, false, 0.0);
        }
    }

    pub fn open_folder(self: &Rc<Self>, folder: &gio::File) {
        self.folder.replace(Some(folder.clone()));
        self.files.set_root(folder);
        self.show_side("files");
        gtk::RecentManager::default().add_item(&folder.uri());
        for name in README_NAMES {
            let child = folder.child(name);
            if child.query_exists(gio::Cancellable::NONE) {
                self.open_file(&child, false, None, true, 0.0);
                return;
            }
        }
        self.close_document();
        let name = basename(folder);
        self.title.set_title(if name.is_empty() { "Amoxtli" } else { &name });
        self.title.set_subtitle(&display_path(folder));
        self.window.set_title(Some(&format!("{name} — Amoxtli")));
        self.content_stack.set_visible_child_name("pick");
    }

    pub fn open_file(self: &Rc<Self>, f: &gio::File, push: bool, fragment: Option<String>, keep_side: bool, scroll: f64) {
        // the edits of the document on screen go to its file before anything else opens
        if !self.flush_save() {
            return;
        }
        let same = self.current.borrow().as_ref().is_some_and(|c| c.equal(f));
        if push && !same {
            if let Some(cur) = self.current.borrow().clone() {
                self.history.borrow_mut().push((cur, self.active_scroller().vadjustment().value()));
                self.forward.borrow_mut().clear();
            }
        }
        let bytes = match f.load_contents(gio::Cancellable::NONE) {
            Ok((bytes, _)) => bytes.to_vec(),
            Err(e) => {
                self.show_error(f, &tr("Unable to Open File"), &format!("{}\n\n{}", display_path(f), e.message()));
                return;
            }
        };
        // A file that isn't UTF-8 is never shown or edited, so it can't be written over.
        let Ok(text) = String::from_utf8(bytes.clone()) else {
            self.show_error(f, &tr("Invalid Encoding"), &format!(
                "{}\n\n{}",
                display_path(f),
                tr("This file isn't UTF-8 text, so Amoxtli can't show or edit it. It hasn't been changed.")
            ));
            return;
        };
        self.current.replace(Some(f.clone()));
        let parent = f.parent();
        self.source.set_text(&text);
        self.disk.replace(Some(bytes));
        self.dirty.set(false);

        // Files panel: the explicit folder if the file lives in it, else the file's directory
        let in_folder = self.folder.borrow().as_ref().is_some_and(|d| f.has_prefix(d));
        if in_folder {
            let d = self.folder.borrow().clone().unwrap();
            self.files.set_root(&d);
        } else if let Some(p) = &parent {
            self.folder.replace(None);
            self.files.set_root(p);
        }

        self.render(&text, parent.clone());
        self.watch(f);
        if !same {
            gtk::RecentManager::default().add_item(&f.uri());
        }
        self.title.set_subtitle(&parent.as_ref().map(display_path).unwrap_or_default());
        self.update_title();
        self.content_stack.set_visible_child_name("doc");
        if !keep_side && self.folder.borrow().is_none() {
            self.show_side("contents");
        }
        self.update_nav();
        self.update_doc_actions();
        self.search_bar.set_search_mode(false);

        if self.mode.get().shows_source() && !keep_side {
            self.source.grab_focus();
        }
        let this = self.clone();
        match fragment {
            Some(frag) => {
                glib::idle_add_local_once(move || this.scroll_to_slug(&frag));
            }
            None => this.restore_scroll(&self.active_scroller().vadjustment(), scroll, 0),
        }
    }

    fn show_error(self: &Rc<Self>, f: &gio::File, title: &str, description: &str) {
        self.current.replace(Some(f.clone()));
        self.disk.replace(None);
        self.dirty.set(false);
        self.source.set_text("");
        self.error_page.set_title(title);
        self.error_page.set_description(Some(description));
        self.content_stack.set_visible_child_name("error");
        self.title.set_title(&basename(f));
        self.title.set_subtitle("");
        self.window.set_title(Some(&format!("{} — Amoxtli", basename(f))));
        self.outline.set_headings(vec![]);
        self.status_bar.set_visible(false);
        self.watch(f); // a fixed file opens by itself
        self.update_doc_actions();
    }

    /// The scroller of the view the person reads: the source in Source mode, else the formatted one.
    fn active_scroller(&self) -> &gtk::ScrolledWindow {
        if self.mode.get() == EditorMode::Source { &self.source_scroller } else { &self.scroller }
    }

    fn update_title(&self) {
        let Some(f) = self.current.borrow().clone() else { return };
        let name = basename(&f);
        // a dot while edits wait to be written, as GNOME apps show unsaved documents
        let shown = if self.dirty.get() { format!("• {name}") } else { name };
        self.title.set_title(&shown);
        self.window.set_title(Some(&format!("{shown} — Amoxtli")));
    }

    /// Shows a text in the formatted editor, keeping its caret where it was.
    fn render(self: &Rc<Self>, text: &str, base: Option<gio::File>) {
        let buf = self.doc.buffer();
        let caret = buf.iter_at_mark(&buf.get_insert()).offset();
        self.doc.render(text, base);
        buf.place_cursor(&buf.iter_at_offset(caret));
        self.matches.borrow_mut().clear();
        self.refresh_outline(text);
        self.status_bar.set_visible(true);
        self.split_scroll.content_changed();
    }

    /// The Contents panel and the word count, from the text as it is now.
    fn refresh_outline(self: &Rc<Self>, text: &str) {
        if let Some(id) = self.outline_source.take() {
            id.remove();
        }
        self.heading_offsets.replace(crate::document::heading_offsets(text));
        self.outline.set_headings(self.doc.headings());
        self.total_words.set(self.doc.count_words(None));
        self.update_stats();
    }

    fn close_document(&self) {
        self.current.replace(None);
        self.disk.replace(None);
        self.dirty.set(false);
        self.source.set_text("");
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        self.doc.render("", None);
        self.outline.set_headings(vec![]);
        self.status_bar.set_visible(false);
        self.update_doc_actions();
    }

    fn reload(self: &Rc<Self>) {
        if self.dirty.get() {
            self.check_disk(); // never drops edits: asks if the file changed
            return;
        }
        let Some(cur) = self.current.borrow().clone() else { return };
        let value = self.active_scroller().vadjustment().value();
        self.open_file(&cur, false, None, true, value);
    }

    /// Layout is validated incrementally; retry until the page is tall enough.
    fn restore_scroll(self: &Rc<Self>, adj: &gtk::Adjustment, value: f64, tries: u32) {
        let adj = adj.clone();
        if value <= 0.0 {
            adj.set_value(0.0);
            return;
        }
        if adj.upper() - adj.page_size() < value && tries < 20 {
            let this = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(25), move || this.restore_scroll(&adj, value, tries + 1));
            return;
        }
        adj.set_value(value);
    }

    // ------------------------------------------------------- live reload

    fn watch(self: &Rc<Self>, f: &gio::File) {
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        let Ok(monitor) = f.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, event| {
            if matches!(event, gio::FileMonitorEvent::AttributeChanged | gio::FileMonitorEvent::PreUnmount) {
                return;
            }
            let Some(w) = weak.upgrade() else { return };
            if let Some(id) = w.reload_source.take() {
                id.remove();
            }
            let weak = Rc::downgrade(&w);
            let id = glib::timeout_add_local_once(Duration::from_millis(180), move || {
                let Some(w) = weak.upgrade() else { return };
                w.reload_source.replace(None);
                w.check_disk();
            });
            w.reload_source.replace(Some(id));
        });
        self.monitor.replace(Some(monitor));
    }

    // ----------------------------------------------------------- editing

    fn set_mode(self: &Rc<Self>, mode: EditorMode) {
        self.mode.set(mode);
        self.action("mode").set_state(&mode.name().to_variant());
        for group in &self.mode_groups {
            group.set_active_name(Some(mode.name()));
        }
        self.source_scroller.set_visible(mode.shows_source());
        self.scroller.set_visible(mode.shows_format());
        self.split_scroll.set_enabled(mode == EditorMode::Split);
        if mode == EditorMode::Split {
            let paned = self.paned.clone();
            glib::idle_add_local_once(move || paned.set_position(paned.width() / 2));
        }
        if mode.shows_format() && self.render_source.borrow().is_some() {
            self.rerender(); // show the latest edits right away
        }
        self.update_editing();
        if self.disk.borrow().is_some() {
            if mode == EditorMode::Format {
                self.flush_rich_sync();
                self.doc.grab_focus();
            } else {
                self.source.grab_focus();
            }
        }
        if self.search_bar.is_search_mode() {
            self.on_search_changed();
        }
        self.update_stats();
    }

    /// The editors take edits, and the tools work, only with a readable file open.
    fn update_editing(&self) {
        let editable = self.disk.borrow().is_some();
        self.source.set_editable(editable);
        self.source.set_cursor_visible(editable);
        self.doc.set_editable(editable);
        self.doc.set_cursor_visible(editable);
        let tools = editable;
        self.format_row.set_visible(tools);
        for tool in Toggle::ALL {
            self.action(tool.action()).set_enabled(tools);
        }
        for name in [STYLE_ACTION, Insert::Table.action(), Insert::HorizontalRule.action()] {
            self.action(name).set_enabled(tools);
        }
        self.action("save").set_enabled(editable);
    }

    /// The editor the tools act on: the one on screen, or in the split view the last one
    /// with focus.
    fn tools_target(&self) -> EditorMode {
        match self.mode.get() {
            EditorMode::Split => self.last_editor.get(),
            mode => mode,
        }
    }

    fn update_format_state(&self) {
        let formats = match self.tools_target() {
            EditorMode::Format => self.doc.active_formats(),
            _ => self.source.active_formats(),
        };
        for tool in Toggle::ALL {
            self.action(tool.action()).set_state(&tool.is_active(&formats).to_variant());
        }
        self.action(STYLE_ACTION).set_state(&editor::style_name(&formats).to_variant());
    }

    fn apply_format(&self, command: &FormatCommand) {
        match self.tools_target() {
            EditorMode::Format => {
                let cell = self.doc.focused_cell();
                self.doc.apply_format(command);
                match cell {
                    Some((_, c)) => {
                        c.grab_focus();
                    }
                    None => {
                        self.doc.grab_focus();
                    }
                }
            }
            _ => {
                self.source.apply_format(command);
                self.source.grab_focus();
            }
        }
        self.update_format_state();
    }

    fn focus_editor(&self) {
        match self.tools_target() {
            EditorMode::Format => self.doc.grab_focus(),
            _ => self.source.grab_focus(),
        };
    }

    fn run_tool(self: &Rc<Self>, tool: Toggle) {
        if let Some(command) = tool.command() {
            self.apply_format(&command);
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let selected = match this.tools_target() {
                EditorMode::Format => this.doc.selected_text(),
                _ => this.source.selected_text(),
            };
            if let Some((text, destination)) = editor::dialogs::ask_link(&this.window, &selected).await {
                this.apply_format(&FormatCommand::Link { text, destination });
            } else {
                this.focus_editor();
            }
        });
    }

    fn insert_table(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            match editor::dialogs::ask_table(&this.window).await {
                Some(dimensions) => this.apply_format(&FormatCommand::Table(dimensions)),
                None => this.focus_editor(),
            }
        });
    }

    fn on_edited(self: &Rc<Self>) {
        if self.disk.borrow().is_none() {
            return;
        }
        self.mark_edited();
        self.split_scroll.content_changed();
        // the formatted view follows 200 ms after the last edit; the Contents and the word count, 250 ms
        if let Some(id) = self.render_source.take() {
            id.remove();
        }
        let delay = if self.mode.get().shows_format() { 200 } else { 250 };
        let weak = Rc::downgrade(self);
        self.render_source.replace(Some(glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            if let Some(w) = weak.upgrade() {
                w.render_source.replace(None);
                w.rerender();
            }
        })));
    }

    /// Before an editor takes edits, it catches up with what the other one has waiting:
    /// otherwise typing on one side and going to the other lost the last edit.
    fn catch_up(self: &Rc<Self>, side: Side) {
        match side {
            Side::Formatted => {
                self.last_editor.set(EditorMode::Format);
                if self.render_source.borrow().is_some() {
                    self.rerender();
                }
            }
            Side::Source => {
                self.last_editor.set(EditorMode::Source);
                self.flush_rich_sync();
            }
        }
        self.update_format_state();
    }

    /// Something changed: the title shows it, and it is saved a second after the last edit.
    fn mark_edited(self: &Rc<Self>) {
        if !self.dirty.replace(true) {
            self.update_title();
        }
        if let Some(id) = self.save_source.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        self.save_source.replace(Some(glib::timeout_add_local_once(Duration::from_secs(1), move || {
            if let Some(w) = weak.upgrade() {
                w.save_source.replace(None);
                w.save_now();
            }
        })));
    }

    /// An edit in the formatted editor: the Markdown follows shortly, and with it the source
    /// editor (only what differs), the Contents panel and the word count.
    fn on_rich_edited(self: &Rc<Self>) {
        if self.disk.borrow().is_none() {
            return;
        }
        self.mark_edited();
        self.split_scroll.content_changed();
        if let Some(id) = self.rich_sync.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        self.rich_sync.replace(Some(glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(w) = weak.upgrade() {
                w.rich_sync.replace(None);
                w.sync_from_rich();
            }
        })));
    }

    fn flush_rich_sync(self: &Rc<Self>) {
        if let Some(id) = self.rich_sync.take() {
            id.remove();
            self.sync_from_rich();
        }
    }

    fn sync_from_rich(self: &Rc<Self>) {
        // a source edit waiting to show in the formatted editor wins: it came later
        if self.render_source.borrow().is_some() {
            return;
        }
        let markdown = self.doc.markdown();
        self.source.apply_external(&markdown);
        if let Some(id) = self.outline_source.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        self.outline_source.replace(Some(glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(w) = weak.upgrade() {
                w.outline_source.replace(None);
                let text = w.source.text();
                w.refresh_outline(&text);
            }
        })));
    }

    /// The text size changed: the formatted editor's spacing follows it.
    pub fn typography_changed(self: &Rc<Self>) {
        self.doc.restyle_all();
        self.split_scroll.content_changed();
    }

    /// Renders the source editor's text again, keeping the formatted view's scroll position.
    fn rerender(self: &Rc<Self>) {
        if let Some(id) = self.render_source.take() {
            id.remove();
        }
        if self.disk.borrow().is_none() {
            return;
        }
        let base = self.current.borrow().as_ref().and_then(|f| f.parent());
        let value = self.scroller.vadjustment().value();
        self.render(&self.source.text(), base);
        self.restore_scroll(&self.scroller.vadjustment(), value, 0);
        if self.search_bar.is_search_mode() && self.mode.get() == EditorMode::Format {
            self.on_search_changed();
        }
    }

    fn restyle(self: &Rc<Self>) {
        self.source.restyle();
        self.doc.restyle_all();
        self.split_scroll.content_changed();
    }

    /// Writes pending edits now. `false` when they couldn't be written.
    fn flush_save(self: &Rc<Self>) -> bool {
        self.save_now()
    }

    fn save_now(self: &Rc<Self>) -> bool {
        if let Some(id) = self.save_source.take() {
            id.remove();
        }
        self.flush_rich_sync();
        if !self.dirty.get() {
            return true;
        }
        let (Some(file), Some(disk)) = (self.current.borrow().clone(), self.disk.borrow().clone()) else { return true };
        let text = self.source.text();
        if text.as_bytes() == disk.as_slice() {
            self.dirty.set(false);
            self.update_title();
            return true;
        }
        // never write over a file that changed in another app
        if let Ok((bytes, _)) = file.load_contents(gio::Cancellable::NONE) {
            if bytes.as_ref() != disk.as_slice() {
                self.resolve_conflict(bytes.to_vec());
                return false;
            }
        }
        // written to a temporary file and moved into place, so a failure leaves the file whole
        match file.replace_contents(text.as_bytes(), None, false, gio::FileCreateFlags::NONE, gio::Cancellable::NONE) {
            Ok(_) => {
                self.disk.replace(Some(text.into_bytes()));
                self.dirty.set(false);
                self.update_title();
                true
            }
            Err(e) => {
                eprintln!("amoxtli: saving {}: {}", display_path(&file), e.message());
                self.toast(
                    &fill(
                        &tr("Couldn’t save “{name}”. Check that you have permission to change it."),
                        &[("name", &basename(&file))],
                    ),
                    5,
                );
                false
            }
        }
    }

    /// Takes in a change made to the file elsewhere: shown right away without edits of
    /// our own, otherwise the person chooses which version to keep.
    fn check_disk(self: &Rc<Self>) {
        let Some(file) = self.current.borrow().clone() else { return };
        let Some(disk) = self.disk.borrow().clone() else {
            // the file couldn't be shown: try again, it may have been fixed
            if file.query_exists(gio::Cancellable::NONE) {
                self.open_file(&file, false, None, true, 0.0);
            }
            return;
        };
        let Ok((bytes, _)) = file.load_contents(gio::Cancellable::NONE) else { return };
        if bytes.as_ref() == disk.as_slice() {
            return; // our own save, or nothing changed
        }
        if self.dirty.get() {
            self.resolve_conflict(bytes.to_vec());
            return;
        }
        match String::from_utf8(bytes.to_vec()) {
            Ok(text) => self.load_external(text),
            Err(_) => {
                let value = self.active_scroller().vadjustment().value();
                self.open_file(&file, false, None, true, value);
            }
        }
    }

    /// Shows the file's new text, keeping the caret and both scroll positions.
    fn load_external(self: &Rc<Self>, text: String) {
        let (caret, _) = self.source.selection();
        let source_scroll = self.source_scroller.vadjustment().value();
        self.source.set_text(&text);
        self.source.place_cursor_at(caret.min(text.chars().count() as i32));
        self.disk.replace(Some(text.into_bytes()));
        self.dirty.set(false);
        self.update_title();
        self.rerender();
        self.restore_scroll(&self.source_scroller.vadjustment(), source_scroll, 0);
    }

    fn resolve_conflict(self: &Rc<Self>, on_disk: Vec<u8>) {
        if self.conflict_open.replace(true) {
            return;
        }
        let Some(file) = self.current.borrow().clone() else { return };
        let readable = String::from_utf8(on_disk.clone()).ok();
        let dialog = adw::AlertDialog::new(
            Some(&tr("File Changed on Disk")),
            Some(&fill(
                &tr("“{name}” was changed in another app. Which version do you want to keep?"),
                &[("name", &basename(&file))],
            )),
        );
        if readable.is_some() {
            dialog.add_response("disk", &tr("Use Disk Version"));
            dialog.set_response_appearance("disk", adw::ResponseAppearance::Destructive);
        }
        dialog.add_response("mine", &tr("Keep Mine"));
        dialog.set_response_appearance("mine", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("mine"));
        dialog.set_close_response("mine");
        let this = self.clone();
        glib::spawn_future_local(async move {
            let response = dialog.choose_future(Some(&this.window)).await;
            this.conflict_open.set(false);
            match readable {
                Some(text) if response == "disk" => this.load_external(text),
                _ => {
                    // what is on disk now is what our version replaces
                    this.disk.replace(Some(on_disk));
                    this.save_now();
                }
            }
        });
    }

    /// Writes pending edits before the window closes; if they can't be written, asks first.
    fn can_close(self: &Rc<Self>) -> bool {
        if !self.dirty.get() || self.save_now() {
            return true;
        }
        if self.conflict_open.get() {
            return false;
        }
        let name = self.current.borrow().as_ref().map(basename).unwrap_or_default();
        let dialog = adw::AlertDialog::new(
            Some(&tr("Unsaved Changes")),
            Some(&fill(&tr("“{name}” couldn’t be saved. Close it anyway and lose your changes?"), &[("name", &name)])),
        );
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("discard", &tr("Discard Changes"))]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let this = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await == "discard" {
                this.dirty.set(false);
                this.window.close();
            }
        });
        false
    }

    // ----------------------------------------------------------- history

    fn go_back(self: &Rc<Self>) {
        let Some((f, scroll)) = self.history.borrow_mut().pop() else { return };
        if let Some(cur) = self.current.borrow().clone() {
            self.forward.borrow_mut().push((cur, self.active_scroller().vadjustment().value()));
        }
        self.open_file(&f, false, None, true, scroll);
    }

    fn go_forward(self: &Rc<Self>) {
        let Some((f, scroll)) = self.forward.borrow_mut().pop() else { return };
        if let Some(cur) = self.current.borrow().clone() {
            self.history.borrow_mut().push((cur, self.active_scroller().vadjustment().value()));
        }
        self.open_file(&f, false, None, true, scroll);
    }

    fn update_nav(&self) {
        let back = !self.history.borrow().is_empty();
        let fwd = !self.forward.borrow().is_empty();
        self.action("back").set_enabled(back);
        self.action("forward").set_enabled(fwd);
        self.nav_box.set_visible(back || fwd);
    }

    // ------------------------------------------------------------- links

    fn on_link(self: &Rc<Self>, url: &str) {
        if let Some(slug) = url.strip_prefix('#') {
            self.scroll_to_slug(slug);
            return;
        }
        let scheme = glib::Uri::peek_scheme(url);
        if scheme.as_deref().is_some_and(|s| s != "file") {
            gtk::UriLauncher::new(url).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
            return;
        }
        let (path, fragment) = match url.split_once('#') {
            Some((p, f)) => (p, Some(f.to_owned()).filter(|f| !f.is_empty())),
            None => (url, None),
        };
        let target = if scheme.is_some() {
            gio::File::for_uri(path)
        } else {
            let path = crate::document::percent_decode(path);
            let base = self.current.borrow().as_ref().and_then(|c| c.parent()).or_else(|| self.folder.borrow().clone());
            let Some(base) = base else { return };
            if path.starts_with('/') { gio::File::for_path(&path) } else { base.resolve_relative_path(&path) }
        };
        if !target.query_exists(gio::Cancellable::NONE) {
            self.toast(&fill(&tr("File not found: {name}"), &[("name", &basename(&target))]), 3);
            return;
        }
        if target.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE) == gio::FileType::Directory {
            let readme = README_NAMES.iter().map(|n| target.child(n)).find(|c| c.query_exists(gio::Cancellable::NONE));
            match readme {
                Some(r) => self.open_file(&r, true, None, false, 0.0),
                None => gtk::FileLauncher::new(Some(&target)).launch(Some(&self.window), gio::Cancellable::NONE, |_| {}),
            }
        } else if is_markdown_name(&basename(&target)) {
            self.open_file(&target, true, fragment, false, 0.0);
        } else {
            gtk::FileLauncher::new(Some(&target)).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
        }
    }

    fn scroll_to_slug(&self, slug: &str) {
        if let Some(h) = self.doc.find_heading(slug) {
            self.scroll_to_heading(&h);
        } else if let Some(mark) = self.doc.find_anchor(slug) {
            self.doc.scroll_to_mark(&mark, 0.0, true, 0.0, 0.0);
        }
    }

    fn scroll_to_heading(&self, h: &Heading) {
        let mode = self.mode.get();
        if mode.shows_source() {
            let index = self.doc.headings().iter().position(|x| x.mark == h.mark);
            if let Some(&offset) = index.and_then(|i| self.heading_offsets.borrow().get(i).copied()).as_ref() {
                self.source.reveal(offset);
            }
        }
        if mode.shows_format() {
            self.doc.scroll_to_mark(&h.mark, 0.0, true, 0.0, 0.0);
        }
        self.outline.highlight(Some(h));
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
    }

    /// In Source mode the Contents panel follows the source editor's scroll position.
    fn on_source_scrolled(&self) {
        if self.mode.get() != EditorMode::Source {
            return;
        }
        let hs = self.doc.headings();
        let Some(first) = hs.first() else { return };
        let buf = self.source.buffer();
        let top = self.source.visible_rect().y() + 60;
        let offsets = self.heading_offsets.borrow();
        let mut current = first;
        for (h, offset) in hs.iter().zip(offsets.iter()) {
            if self.source.iter_location(&buf.iter_at_offset(*offset)).y() <= top {
                current = h;
            } else {
                break;
            }
        }
        self.outline.highlight(Some(current));
    }

    fn on_scrolled(&self) {
        if self.mode.get() == EditorMode::Source {
            return;
        }
        let hs = self.doc.headings();
        let Some(first) = hs.first() else { return };
        let adj = self.scroller.vadjustment();
        let y = adj.value() as i32 + 60;
        let mut current = first;
        for h in &hs {
            if self.doc.heading_y(h) <= y {
                current = h;
            } else {
                break;
            }
        }
        if adj.value() >= adj.upper() - adj.page_size() - 2.0 {
            // at the very bottom, highlight the last visible heading
            let bottom = (adj.value() + adj.page_size()) as i32;
            if let Some(last) = hs.iter().rev().find(|h| self.doc.heading_y(h) <= bottom) {
                current = last;
            }
        }
        self.outline.highlight(Some(current));
    }

    // ------------------------------------------------------------ search

    /// Search looks in the source while it is on screen, else in the formatted document.
    fn search_view(&self) -> gtk::TextView {
        if self.mode.get().shows_source() && self.disk.borrow().is_some() {
            self.source.clone().upcast()
        } else {
            self.doc.clone().upcast()
        }
    }

    fn clear_matches(&self) {
        for buf in [self.doc.buffer(), self.source.buffer()] {
            let (start, end) = buf.bounds();
            for name in ["match", "match-current"] {
                if let Some(tag) = buf.tag_table().lookup(name) {
                    buf.remove_tag(&tag, &start, &end);
                }
            }
        }
        self.matches.borrow_mut().clear();
        self.match_index.set(-1);
    }

    fn on_search_changed(&self) {
        self.clear_matches();
        let needle = self.search_entry.text();
        if needle.is_empty() {
            self.match_label.set_label("");
            return;
        }
        let view = self.search_view();
        let buf = view.buffer();
        let Some(tag) = buf.tag_table().lookup("match") else { return };
        let mut it = buf.start_iter();
        let flags = gtk::TextSearchFlags::CASE_INSENSITIVE | gtk::TextSearchFlags::TEXT_ONLY;
        let mut found = vec![];
        while let Some((s, e)) = it.forward_search(&needle, flags, None) {
            buf.apply_tag(&tag, &s, &e);
            found.push((s.offset(), e.offset()));
            it = e;
        }
        if found.is_empty() {
            self.match_label.set_label(&tr("No results"));
            return;
        }
        // first match after the current scroll position
        let top = view.visible_rect().y();
        let idx = found
            .iter()
            .position(|(s, _)| view.iter_location(&buf.iter_at_offset(*s)).y() >= top)
            .unwrap_or(0);
        self.matches.replace(found);
        self.select_match(idx as i32);
    }

    fn step_match(&self, delta: i32) {
        let n = self.matches.borrow().len() as i32;
        if n == 0 {
            return;
        }
        self.select_match((self.match_index.get() + delta).rem_euclid(n));
    }

    fn select_match(&self, idx: i32) {
        let view = self.search_view();
        let buf = view.buffer();
        let Some(cur) = buf.tag_table().lookup("match-current") else { return };
        let matches = self.matches.borrow();
        if let Some((s, e)) = matches.get(self.match_index.get().max(0) as usize).filter(|_| self.match_index.get() >= 0) {
            buf.remove_tag(&cur, &buf.iter_at_offset(*s), &buf.iter_at_offset(*e));
        }
        self.match_index.set(idx);
        let (s, e) = matches[idx as usize];
        let mut si = buf.iter_at_offset(s);
        buf.apply_tag(&cur, &si, &buf.iter_at_offset(e));
        view.scroll_to_iter(&mut si, 0.15, true, 0.0, 0.35);
        self.match_label.set_label(&fill(
            &tr("{current} of {total}"),
            &[("current", &(idx + 1).to_string()), ("total", &matches.len().to_string())],
        ));
    }

    // ------------------------------------------- word count / reading time

    fn reading_time(words: u64) -> String {
        let minutes = words as f64 / WORDS_PER_MINUTE;
        if minutes < 1.0 {
            return if words > 0 { tr("less than 1 min read") } else { tr("0 min read") };
        }
        let total = minutes.ceil() as u64;
        if total < 60 {
            return fill(&trn("{n} min read", "{n} min read", total), &[("n", &total.to_string())]);
        }
        let (h, m) = (total / 60, total % 60);
        if m > 0 {
            fill(&tr("{hours} h {minutes} min read"), &[("hours", &h.to_string()), ("minutes", &m.to_string())])
        } else {
            fill(&tr("{hours} h read"), &[("hours", &h.to_string())])
        }
    }

    fn queue_stats(self: &Rc<Self>) {
        if self.stats_source.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(80), move || {
            if let Some(w) = weak.upgrade() {
                w.stats_source.replace(None);
                w.update_stats();
            }
        });
        self.stats_source.replace(Some(id));
    }

    fn update_stats(&self) {
        let total = self.total_words.get();
        let selected = if self.mode.get().shows_source() && self.source.buffer().has_selection() {
            Some(crate::document::count_words(&self.source.selected_text()) as u64)
        } else {
            self.doc.buffer().selection_bounds().map(|range| self.doc.count_words(Some(range)))
        };
        match selected {
            Some(sel) => {
                self.words_label.set_label(&fill(
                    &trn("{selected} of {total} word", "{selected} of {total} words", total),
                    &[("selected", &group_digits(sel)), ("total", &group_digits(total))],
                ));
                self.reading_label.set_label(&Self::reading_time(sel));
            }
            None => {
                self.words_label.set_label(&fill(&trn("{n} word", "{n} words", total), &[("n", &group_digits(total))]));
                self.reading_label.set_label(&Self::reading_time(total));
            }
        }
    }

    // -------------------------------------------------------------- misc

    #[cfg(debug_assertions)]
    pub fn debug_actions(self: &Rc<Self>) {
        if let Some(n) = std::env::var("AMOXTLI_SNAPSHOT_HEADING").ok().and_then(|n| n.parse::<usize>().ok()) {
            if let Some(h) = self.doc.headings().get(n) {
                self.scroll_to_heading(h);
            }
        }
        if let Some(mode) = std::env::var("AMOXTLI_SNAPSHOT_MODE").ok().as_deref().and_then(EditorMode::from_name) {
            self.set_mode(mode);
        }
        if let Some(n) = std::env::var("AMOXTLI_SNAPSHOT_CARET").ok().and_then(|n| n.parse::<i32>().ok()) {
            self.source.place_cursor_at(n);
        }
        if let Some(y) = std::env::var("AMOXTLI_SNAPSHOT_SCROLL").ok().and_then(|y| y.parse::<f64>().ok()) {
            // once laid out, scroll the source and let the split view follow
            let this = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(800), move || {
                this.source_scroller.vadjustment().set_value(y);
                this.split_scroll.follow(Side::Source);
            });
        }
        if std::env::var("AMOXTLI_SELFTEST").is_ok() {
            self.self_test();
        }
        if std::env::var("AMOXTLI_SNAPSHOT_EXPAND").is_ok() {
            self.files.expand_top_level();
        }
        if let Ok(text) = std::env::var("AMOXTLI_SNAPSHOT_SEARCH") {
            self.search_bar.set_search_mode(true);
            self.search_entry.set_text(&text);
        }
        if let Ok(list) = std::env::var("AMOXTLI_STRESS") {
            let start = std::time::Instant::now();
            let mut n = 0;
            for path in std::fs::read_to_string(list).unwrap_or_default().lines().filter(|l| !l.is_empty()) {
                let f = gio::File::for_path(path);
                if let Ok((bytes, _)) = f.load_contents(gio::Cancellable::NONE) {
                    let text = String::from_utf8_lossy(&bytes);
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.doc.render(&text, f.parent())));
                    if r.is_err() {
                        println!("PANIC {path}");
                    }
                    n += 1;
                }
            }
            println!("stress: rendered {n} files, avg {:.1} ms", start.elapsed().as_secs_f64() * 1000.0 / n.max(1) as f64);
        }
        if let Some(n) = std::env::var("AMOXTLI_SNAPSHOT_SELECT").ok().and_then(|n| n.parse::<i32>().ok()) {
            let buf = self.doc.buffer();
            buf.select_range(&buf.start_iter(), &buf.iter_at_offset(n));
        }
    }

    /// Drives the editor the way typing would, and checks the file on disk. It writes to the
    /// open file: run it on a scratch copy.
    #[cfg(debug_assertions)]
    fn self_test(self: &Rc<Self>) {
        use crate::markdown::syntax;
        let mut failures = 0;
        let mut check = |name: &str, ok: bool, detail: String| {
            println!("{} {name}{}", if ok { "PASS" } else { "FAIL" }, if ok { String::new() } else { format!(": {detail}") });
            failures += usize::from(!ok);
        };

        self.set_mode(EditorMode::Source);
        let src = self.source.clone();
        let buf = src.buffer();
        let type_text = |t: &str| {
            buf.insert_interactive_at_cursor(t, true);
        };
        let load = |t: &str| {
            src.set_text(t);
            buf.place_cursor(&buf.end_iter());
        };

        load("1. uno");
        src.press_return();
        type_text("dos");
        src.press_return();
        src.press_return();
        type_text("fin");
        check("return continues and ends a list", src.text() == "1. uno\n2. dos\nfin", src.text());

        load("");
        src.apply_format(&FormatCommand::BulletList);
        type_text("uno");
        src.press_return();
        type_text("dos");
        check("list started with the tool continues", src.text() == "- uno\n- dos", src.text());

        load("> - ");
        check("empty item in a quote", src.press_return() && src.text() == "> ", src.text());
        load("```\n- code");
        check("return in a code block is a line break", !src.press_return(), src.text());

        load("hola");
        buf.select_range(&buf.start_iter(), &buf.end_iter());
        src.apply_format(&FormatCommand::Bold);
        let bold = src.text();
        buf.undo();
        check("a tool is one undo step", bold == "**hola**" && src.text() == "hola", format!("{bold:?} then {:?}", src.text()));
        check("undo can't go back past opening", { buf.undo(); src.text() == "hola" }, src.text());

        let code = buf.tag_table().lookup("code").unwrap();
        load("a\n*x*\nb");
        let mut start = buf.start_iter();
        buf.insert(&mut start, "```\n");
        let x_line = buf.iter_at_line(2).unwrap();
        check("opening a fence styles what follows", x_line.has_tag(&code), String::new());
        let (mut s, mut e) = (buf.start_iter(), buf.iter_at_line(1).unwrap());
        buf.delete(&mut s, &mut e);
        let x_line = buf.iter_at_line(1).unwrap();
        check("removing the fence restyles what follows", !x_line.has_tag(&code), String::new());
        check("fence helper agrees", syntax::is_fence("```rust"), String::new());

        let marker = buf.tag_table().lookup("list-marker").unwrap();
        load("# T\n\n## L\n\n- one\n- two");
        let first = buf.iter_at_line(4).unwrap();
        let second = buf.iter_at_line(5).unwrap();
        check("every list marker is colored", first.has_tag(&marker) && second.has_tag(&marker),
            format!("{} {}", first.has_tag(&marker), second.has_tag(&marker)));

        load("a **bold** b");
        src.place_cursor_at(5);
        let f = src.active_formats();
        check("active formats at the caret", f.strong && !f.emphasis, format!("{f:?}"));

        use crate::application::{current_font, current_size};
        use crate::typography::{ReadingFont, TextSize};
        let app = self.window.application().unwrap();
        let (font, size) = (current_font(), current_size());
        app.activate_action("text-larger", None);
        let larger = current_size();
        app.activate_action("font", Some(&"serif".to_variant()));
        let serif = current_font();
        app.activate_action("text-reset", None);
        check("text size and font change live", larger == size.larger().unwrap_or(size) && serif == ReadingFont::Serif
            && current_size() == TextSize::STANDARD, format!("{larger:?} {serif:?} {:?}", current_size()));
        app.activate_action("font", Some(&font.name().to_variant()));
        let _ = size;

        self.rich_self_test(&mut check);

        // an edit in the formatted editor reaches the Markdown text, and the file
        self.set_mode(EditorMode::Split);
        self.source.set_text("# T\n\nbody\n");
        self.rerender();
        let rb = self.doc.buffer();
        rb.place_cursor(&rb.end_iter());
        rb.insert_interactive_at_cursor("!", true);
        self.flush_rich_sync();
        check("split: a formatted edit reaches the source", self.source.text() == "# T\n\nbody!\n", self.source.text());
        let shown = self.mode_groups[0].active_name().map(|n| n.to_string()).unwrap_or_default();
        check("split: the mode selector shows it", shown == "split", shown);
        self.source.buffer().insert_interactive_at_cursor("x", true);
        self.rerender();
        check("split: a source edit reaches the formatted editor", self.doc.markdown().contains('x'), self.doc.markdown());

        // switching sides never loses the last edit
        self.source.set_text("one\n");
        self.rerender();
        self.source.buffer().place_cursor(&self.source.buffer().end_iter());
        self.source.buffer().insert_interactive_at_cursor("\ntwo\n", true);
        self.catch_up(Side::Formatted);
        let rb = self.doc.buffer();
        rb.place_cursor(&rb.end_iter());
        self.doc.press_return();
        rb.insert_interactive_at_cursor("three", true);
        self.catch_up(Side::Source);
        check("split: both sides keep the other's last edit", self.source.text() == "one\n\ntwo\n\nthree\n", format!("{:?}", self.source.text()));

        self.set_mode(EditorMode::Source);

        // saving: written atomically, and never over a file changed elsewhere
        let Some(file) = self.current.borrow().clone() else { return };
        let read = || file.load_contents(gio::Cancellable::NONE).map(|(b, _)| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        load("before");
        self.disk.replace(Some(read().into_bytes()));
        type_text(" edited");
        check("typing marks the document edited", self.dirty.get(), String::new());
        check("save writes the file", self.flush_save() && read() == "before edited" && !self.dirty.get(), read());
        let _ = std::fs::write(file.path().unwrap(), "changed elsewhere");
        type_text("!");
        check("a changed file isn't written over", !self.save_now() && read() == "changed elsewhere" && self.conflict_open.get(), read());
        self.dirty.set(false);
        println!("self-test: {failures} failed");
        self.scroll_self_test();
    }

    /// Scrolling one side of the split view brings the other to the same block. Run once both
    /// sides are laid out: set `AMOXTLI_SNAPSHOT_DELAY` to wait for it.
    #[cfg(debug_assertions)]
    fn scroll_self_test(self: &Rc<Self>) {
        // the save checks changed the file: its reload mustn't replace the text measured here
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        if let Some(id) = self.reload_source.take() {
            id.remove();
        }
        self.disk.replace(None);
        self.set_mode(EditorMode::Split);
        let long: String = (0..30).map(|i| format!("## Part {i}\n\nSome text, **bold** too.\n\n- a\n- b\n\n```\ncode {i}\nmore\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n")).collect();
        self.source.set_text(&long);
        self.disk.replace(Some(long.into_bytes()));
        self.rerender();
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            let report = |name: &str, ok: bool, detail: String| println!("{} {name}{}", if ok { "PASS" } else { "FAIL" }, if ok { String::new() } else { format!(": {detail}") });
            let map = this.split_scroll.current_map();
            report("split: the blocks of both sides pair up", map.points.len() > 100, format!("{} points", map.points.len()));
            let p = map.points[map.points.len() / 2];
            this.source_scroller.vadjustment().set_value(p.source);
            this.split_scroll.follow(Side::Source);
            let rich = this.scroller.vadjustment().value();
            this.scroller.vadjustment().set_value(map.points[map.points.len() / 3].formatted);
            this.split_scroll.follow(Side::Formatted);
            let source = this.source_scroller.vadjustment().value();
            let q = map.points[map.points.len() / 3];
            report("split: scrolling either side lines the other up", p.source > 0.0 && (rich - p.formatted).abs() < 1.0 && (source - q.source).abs() < 1.0,
                format!("{rich} vs {}, {source} vs {}", p.formatted, q.source));
            // halfway between two blocks on one side is halfway on the other
            let (a, b) = (map.points[10], map.points[11]);
            this.source_scroller.vadjustment().set_value((a.source + b.source) / 2.0);
            this.split_scroll.follow(Side::Source);
            let mid = this.scroller.vadjustment().value();
            report("split: between blocks, positions are interpolated", (mid - (a.formatted + b.formatted) / 2.0).abs() < 1.0, format!("{mid}"));
            println!("scroll-test: done");
        });
    }

    /// The formatted editor, driven as typing would drive it.
    #[cfg(debug_assertions)]
    fn rich_self_test(self: &Rc<Self>, check: &mut impl FnMut(&str, bool, String)) {
        use crate::markdown::format::TableDimensions;
        use gtk::gdk::{Key, ModifierType};
        let doc = self.doc.clone();
        let buf = doc.buffer();
        let none = ModifierType::empty();
        let load = |md: &str| {
            doc.render(md, None);
            buf.place_cursor(&buf.end_iter());
            doc.update_typing_style();
        };
        let type_text = |t: &str| {
            buf.insert_interactive_at_cursor(t, true);
        };
        let md = || doc.markdown();
        let at_line_start = |line: i32| buf.place_cursor(&buf.iter_at_line(line).unwrap());

        load("- one");
        doc.press_return();
        type_text("two");
        check("rich: Return makes another item", md() == "- one\n- two\n", md());
        doc.press_return();
        doc.press_return();
        type_text("after");
        check("rich: Return on an empty item ends the list", md() == "- one\n- two\n\nafter\n", md());

        load("# Title");
        doc.press_return();
        type_text("body");
        check("rich: Return after a heading is body text", md() == "# Title\n\nbody\n", md());

        load("- a\n- b");
        at_line_start(1);
        doc.key(Key::BackSpace, none);
        check("rich: Delete at an item's start removes the bullet", md() == "- a\n\nb\n", md());
        load("text\n\n## Head");
        at_line_start(1);
        doc.key(Key::BackSpace, none);
        check("rich: Delete at a heading's start makes it body text", md() == "text\n\nHead\n", md());

        load("- a\n- b");
        at_line_start(1);
        doc.key(Key::Tab, none);
        let nested = md();
        doc.key(Key::ISO_Left_Tab, ModifierType::SHIFT_MASK);
        check("rich: Tab nests an item and Shift+Tab takes it out", nested == "- a\n  - b\n" && md() == "- a\n- b\n", format!("{nested:?} {:?}", md()));

        load("```\nlet a");
        doc.press_return();
        doc.press_return();
        type_text("b");
        check("rich: Return in code adds lines, blank ones too", md() == "```\nlet a\n\nb\n```\n", md());
        load("```\nx\n```");
        doc.press_return();
        let stays = doc.kind_at_caret().block.is_code();
        type_text("y");
        check("rich: the caret stays in the block after Return at the end", stays && md() == "```\nx\ny\n```\n", md());
        load("```\nx\n```");
        doc.key(Key::Down, none);
        type_text("out");
        check("rich: Down leaves a code block at the end", md() == "```\nx\n```\n\nout\n", md());

        load("## Gone");
        buf.select_range(&buf.iter_at_line(0).unwrap(), &buf.end_iter());
        buf.delete_interactive(&mut buf.iter_at_line(0).unwrap(), &mut buf.end_iter(), true);
        type_text("plain");
        check("rich: a heading emptied becomes body text", md() == "plain\n", md());

        load("");
        for c in ["#", "#", " ", "T"] {
            type_text(c);
        }
        type_text("itle");
        check("rich: typing ## makes a heading", md() == "## Title\n", md());
        load("");
        for c in ["\\", "#", " ", "x"] {
            type_text(c);
        }
        check("rich: \\# stays a literal hash", md() == "\\# x\n" && doc.kind_at_caret().block == crate::markdown::model::Block::Paragraph, md());
        load("\\# loaded");
        buf.place_cursor(&buf.end_iter());
        type_text("!");
        check("rich: a loaded hash stays literal", md() == "\\# loaded!\n", md());

        load("word");
        buf.select_range(&buf.start_iter(), &buf.end_iter());
        doc.apply_format(&FormatCommand::Bold);
        let bold = md();
        let pressed = doc.active_formats().strong;
        doc.apply_format(&FormatCommand::Bold);
        check("rich: bold toggles on the selection and shows pressed", bold == "**word**\n" && pressed && md() == "word\n", format!("{bold:?} {pressed}"));
        load("a");
        type_text(" ");
        doc.apply_format(&FormatCommand::Italic);
        let pressed = doc.active_formats().emphasis;
        type_text("b");
        check("rich: italic with no selection applies to what's typed", pressed && md() == "a *b*\n", md());

        for (command, expected) in [
            (FormatCommand::Heading(2), "## x\n"),
            (FormatCommand::Quote, "> x\n"),
            (FormatCommand::BulletList, "- x\n"),
            (FormatCommand::NumberedList, "1. x\n"),
            (FormatCommand::CodeBlock, "```\nx\n```\n"),
        ] {
            load("x");
            doc.apply_format(&command);
            let on = md();
            doc.apply_format(&command);
            check(&format!("rich: {command:?} applies and toggles off"), on == expected && md() == "x\n", format!("{on:?} then {:?}", md()));
        }
        load("see");
        buf.select_range(&buf.start_iter(), &buf.end_iter());
        doc.apply_format(&FormatCommand::Link { text: String::new(), destination: "https://a.b".into() });
        check("rich: link on the selection", md() == "[see](https://a.b)\n", md());
        load("text");
        doc.apply_format(&FormatCommand::HorizontalRule);
        type_text("below");
        check("rich: a rule goes below with a line to type on", md() == "text\n\n---\n\nbelow\n", md());
        load("");
        let dims = TableDimensions::new(2, 2, |n| format!("H{n}")).unwrap();
        doc.apply_format(&FormatCommand::Table(dims));
        check("rich: a table inserts its header and rows", md() == "| H1 | H2 |\n| --- | --- |\n|  |  |\n", md());

        load("- a\n- b");
        doc.press_return();
        let markers = doc.line_markers();
        check("rich: the empty last line shows the next bullet", markers.last().is_some_and(|m| m.len() == 1),
            format!("{:?}", markers.last()));
        load("1. a\n2. b");
        doc.press_return();
        let markers = doc.line_markers();
        check("rich: and the next number", markers.last().and_then(|m| m.first()).is_some_and(|m| m.text == "3."),
            format!("{:?}", markers.last()));

        load("one");
        type_text(" two");
        doc.apply_format(&FormatCommand::Heading(1));
        doc.undo();
        let after_undo = md();
        doc.undo();
        check("rich: undo restores tags and text, step by step", after_undo == "one two\n" && md() == "one\n", format!("{after_undo:?} then {:?}", md()));
        doc.redo();
        check("rich: redo", md() == "one two\n", md());

        let sample = "# Doc\n\nSome **bold** and [a link](u).\n\n> [!TIP]\n> Alert\n\n| A | B |\n| :--- | ---: |\n| 1 |  |\n\n- [ ] task\n\n<div>raw</div>\n";
        load(sample);
        check("rich: what opens comes back unchanged", md() == sample, md());
        if let Some((_, table)) = doc.imp_tables().first().cloned() {
            let cell = &table.cells[1][1];
            cell.buffer().insert_interactive_at_cursor("2", true);
            check("rich: typing in a cell", md().contains("| 1 | 2 |"), md());
        }
    }

    fn show_side(&self, name: &str) {
        self.side_toggles.set_active_name(Some(name));
    }

    pub fn toast(&self, text: &str, timeout: u32) {
        self.toasts.add_toast(adw::Toast::builder().title(text).timeout(timeout).build());
    }

    pub fn refresh_banner(&self) {
        let st = State::load();
        // a sandboxed app can't change the host's default apps
        let installed = !config::is_flatpak() && config::desktop_app_info().is_some();
        let is_default = gio::AppInfo::default_for_type("text/markdown", false)
            .and_then(|a| a.id())
            .is_some_and(|id| id == format!("{APP_ID}.desktop"));
        self.banner.set_revealed(installed && !is_default && !st.banner_dismissed);
    }

    fn edit_externally(&self) {
        let Some(cur) = self.current.borrow().clone() else { return };
        if config::is_flatpak() {
            // host apps aren't visible in the sandbox: let the portal show the app chooser
            let launcher = gtk::FileLauncher::new(Some(&cur));
            launcher.set_always_ask(true);
            launcher.launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
            return;
        }
        let editor = gio::AppInfo::default_for_type("text/plain", false)
            .filter(|a| a.id().is_none_or(|id| id != format!("{APP_ID}.desktop")));
        match editor {
            Some(app) => {
                let ctx = WidgetExt::display(&self.window).app_launch_context();
                if let Err(e) = app.launch(&[cur], Some(&ctx)) {
                    self.toast(e.message(), 3);
                }
            }
            None => gtk::FileLauncher::new(Some(&cur)).launch(Some(&self.window), gio::Cancellable::NONE, |_| {}),
        }
    }

    fn show_in_files(&self) {
        if let Some(cur) = self.current.borrow().clone() {
            gtk::FileLauncher::new(Some(&cur)).open_containing_folder(Some(&self.window), gio::Cancellable::NONE, |_| {});
        }
    }

    fn copy_path(&self) {
        if let Some(cur) = self.current.borrow().clone() {
            let text = cur.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| cur.uri().to_string());
            self.window.clipboard().set_text(&text);
            self.toast(&tr("Path copied"), 3);
        }
    }

    fn on_close(&self) {
        let mut st = State::load();
        if self.banner.is_revealed() {
            st.banner_dismissed = true; // only nag once
        }
        st.mode = self.mode.get().name().into();
        st.sidebar = self.split.shows_sidebar();
        st.maximized = self.window.is_maximized();
        if !st.maximized {
            let (w, h) = self.window.default_size();
            st.width = w;
            st.height = h;
        }
        st.save();
        let sm = adw::StyleManager::default();
        for id in self.style_handlers.take() {
            sm.disconnect(id);
        }
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        for source in [self.render_source.take(), self.save_source.take(), self.reload_source.take(), self.rich_sync.take(), self.outline_source.take()]
            .into_iter()
            .flatten()
        {
            source.remove();
        }
        let me = self.window.clone();
        WINDOWS.with(|w| w.borrow_mut().retain(|x| x.window != me));
    }
}

/// The Source / Format / Split selector, with labels or, for narrow windows, icons.
fn mode_toggles(icons: bool) -> adw::ToggleGroup {
    let group = adw::ToggleGroup::builder().valign(gtk::Align::Center).build();
    let modes = [
        (EditorMode::Source, tr("Source"), "amoxtli-source-symbolic", tr("Show the Markdown source")),
        (EditorMode::Format, tr("Format"), "format-text-rich-symbolic", tr("Show the formatted document")),
        (EditorMode::Split, tr("Split"), "view-dual-symbolic", tr("Show the source and the formatted document side by side")),
    ];
    for (mode, label, icon, help) in modes {
        let keys = editor::accel_label(mode_accel(mode)).unwrap_or_default();
        let toggle = adw::Toggle::builder().name(mode.name()).tooltip(format!("{help} ({keys})"));
        let toggle = if icons { toggle.icon_name(icon) } else { toggle.label(label) };
        group.add(toggle.build());
    }
    group
}

pub fn mode_accel(mode: EditorMode) -> &'static str {
    match mode {
        EditorMode::Source => "<Ctrl><Alt>3",
        EditorMode::Format => "<Ctrl><Alt>4",
        EditorMode::Split => "<Ctrl><Alt>5",
    }
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let section = |items: &[(String, &str)]| {
        let s = gio::Menu::new();
        for (label, action) in items {
            s.append(Some(label), Some(action));
        }
        menu.append_section(None, &s);
    };
    section(&[(tr("New Window"), "app.new-window"), (tr("Find"), "win.find"), (tr("Reload"), "win.reload")]);
    let text = crate::application::size_menu();
    text.append_submenu(Some(&tr("Font")), &crate::application::font_menu());
    menu.append_section(None, &text);
    section(&[
        (tr("Edit in Text Editor"), "win.edit"),
        (tr("Show in Files"), "win.show-in-files"),
        (tr("Copy File Path"), "win.copy-path"),
    ]);
    let mut last = vec![(tr("Keyboard Shortcuts"), "app.shortcuts"), (tr("About Amoxtli"), "app.about")];
    if !config::is_flatpak() {
        last.insert(0, (tr("Set as Default Markdown Viewer"), "app.make-default"));
    }
    section(&last);
    menu
}
