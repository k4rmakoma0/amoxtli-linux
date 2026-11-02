//! The main window: header bar, split view, search, history, live reload, status bar.

use crate::config::{self, State, APP_ID, MIME_TYPES};
use crate::document::{DocumentView, Heading};
use crate::i18n::{fill, group_digits, tr, trn};
use crate::sidebar::{is_markdown_name, FilesPanel, OutlinePanel, MARKDOWN_SUFFIXES};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const ZOOM_STEPS: [i32; 26] = [
    50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230, 240, 250,
    260, 270, 280, 290, 300,
];
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
    doc: DocumentView,
    scroller: gtk::ScrolledWindow,
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
    zoom: Cell<i32>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_source: RefCell<Option<glib::SourceId>>,
    stats_source: RefCell<Option<glib::SourceId>>,
    matches: RefCell<Vec<(i32, i32)>>,
    match_index: Cell<i32>,
    total_words: Cell<u64>,
    style_handlers: RefCell<Vec<glib::SignalHandlerId>>,
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
        let doc = DocumentView::new();
        let clamp = adw::ClampScrollable::builder().child(&doc).maximum_size(920).tightening_threshold(640).build();
        let scroller = gtk::ScrolledWindow::builder().child(&clamp).vexpand(true).build();

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
        content_stack.add_named(&scroller, Some("doc"));
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
        window.add_breakpoint(bp);

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
            zoom: Cell::new(st.zoom),
            monitor: RefCell::new(None),
            reload_source: RefCell::new(None),
            stats_source: RefCell::new(None),
            matches: RefCell::new(vec![]),
            match_index: Cell::new(-1),
            total_words: Cell::new(0),
            style_handlers: RefCell::new(vec![]),
        });
        WINDOWS.with(|w| w.borrow_mut().push(this.clone()));
        this.connect_signals(&prev_b, &next_b);
        this.setup_actions();
        this.update_nav();
        this.refresh_banner();
        this.apply_zoom();
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
                    if w.current.borrow().is_some() {
                        w.reload();
                    }
                }
            }
        };
        self.style_handlers.replace(vec![
            sm.connect_dark_notify(restyle.clone()),
            sm.connect_accent_color_notify(restyle),
        ]);

        self.window.connect_close_request({
            let weak = weak.clone();
            move |_| {
                if let Some(w) = weak.upgrade() {
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
        add("zoom-in", |w| w.step_zoom(1));
        add("zoom-out", |w| w.step_zoom(-1));
        add("zoom-reset", |w| w.set_zoom(100));
        add("back", |w| w.go_back());
        add("forward", |w| w.go_forward());
        add("edit", |w| w.edit_externally());
        add("show-in-files", |w| w.show_in_files());
        add("copy-path", |w| w.copy_path());
        add("close", |w| w.window.close());
        add("top", |w| w.scroller.vadjustment().set_value(0.0));
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
        let same = self.current.borrow().as_ref().is_some_and(|c| c.equal(f));
        if push && !same {
            if let Some(cur) = self.current.borrow().clone() {
                self.history.borrow_mut().push((cur, self.scroller.vadjustment().value()));
                self.forward.borrow_mut().clear();
            }
        }
        let text = match f.load_contents(gio::Cancellable::NONE) {
            Ok((bytes, _)) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                self.error_page.set_description(Some(&format!("{}\n\n{}", display_path(f), e.message())));
                self.content_stack.set_visible_child_name("error");
                self.title.set_title(&basename(f));
                self.title.set_subtitle("");
                return;
            }
        };
        self.current.replace(Some(f.clone()));
        let parent = f.parent();

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
        let name = basename(f);
        self.title.set_title(&name);
        self.title.set_subtitle(&parent.as_ref().map(display_path).unwrap_or_default());
        self.window.set_title(Some(&format!("{name} — Amoxtli")));
        self.content_stack.set_visible_child_name("doc");
        if !keep_side && self.folder.borrow().is_none() {
            self.show_side("contents");
        }
        self.update_nav();
        self.update_doc_actions();
        self.search_bar.set_search_mode(false);

        let this = self.clone();
        match fragment {
            Some(frag) => {
                glib::idle_add_local_once(move || this.scroll_to_slug(&frag));
            }
            None => this.restore_scroll(scroll, 0),
        }
    }

    fn render(self: &Rc<Self>, text: &str, base: Option<gio::File>) {
        self.doc.render(text, base);
        self.outline.set_headings(self.doc.headings());
        self.matches.borrow_mut().clear();
        self.total_words.set(self.doc.count_words(None));
        let buf = self.doc.buffer();
        let weak = Rc::downgrade(self);
        buf.connect_has_selection_notify({
            let weak = weak.clone();
            move |_| {
                if let Some(w) = weak.upgrade() {
                    w.queue_stats();
                }
            }
        });
        buf.connect_mark_set(move |buf, _, mark| {
            if mark == &buf.get_insert() || mark == &buf.selection_bound() {
                if let Some(w) = weak.upgrade() {
                    w.queue_stats();
                }
            }
        });
        self.update_stats();
        self.status_bar.set_visible(true);
    }

    fn close_document(&self) {
        self.current.replace(None);
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        self.doc.render("", None);
        self.outline.set_headings(vec![]);
        self.status_bar.set_visible(false);
        self.update_doc_actions();
    }

    fn reload(self: &Rc<Self>) {
        let Some(cur) = self.current.borrow().clone() else { return };
        let value = self.scroller.vadjustment().value();
        self.open_file(&cur, false, None, true, value);
    }

    /// Layout is validated incrementally; retry until the page is tall enough.
    fn restore_scroll(self: &Rc<Self>, value: f64, tries: u32) {
        let adj = self.scroller.vadjustment();
        if value <= 0.0 {
            adj.set_value(0.0);
            return;
        }
        if adj.upper() - adj.page_size() < value && tries < 20 {
            let this = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(25), move || this.restore_scroll(value, tries + 1));
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
                let exists = w.current.borrow().as_ref().is_some_and(|c| c.query_exists(gio::Cancellable::NONE));
                if exists {
                    w.reload();
                }
            });
            w.reload_source.replace(Some(id));
        });
        self.monitor.replace(Some(monitor));
    }

    // ----------------------------------------------------------- history

    fn go_back(self: &Rc<Self>) {
        let Some((f, scroll)) = self.history.borrow_mut().pop() else { return };
        if let Some(cur) = self.current.borrow().clone() {
            self.forward.borrow_mut().push((cur, self.scroller.vadjustment().value()));
        }
        self.open_file(&f, false, None, true, scroll);
    }

    fn go_forward(self: &Rc<Self>) {
        let Some((f, scroll)) = self.forward.borrow_mut().pop() else { return };
        if let Some(cur) = self.current.borrow().clone() {
            self.history.borrow_mut().push((cur, self.scroller.vadjustment().value()));
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
        self.doc.scroll_to_mark(&h.mark, 0.0, true, 0.0, 0.0);
        self.outline.highlight(Some(h));
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
    }

    fn on_scrolled(&self) {
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

    fn clear_matches(&self) {
        let buf = self.doc.buffer();
        let (start, end) = buf.bounds();
        for name in ["match", "match-current"] {
            if let Some(tag) = buf.tag_table().lookup(name) {
                buf.remove_tag(&tag, &start, &end);
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
        let buf = self.doc.buffer();
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
        let top = self.scroller.vadjustment().value() as i32;
        let idx = found
            .iter()
            .position(|(s, _)| self.doc.iter_location(&buf.iter_at_offset(*s)).y() >= top)
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
        let buf = self.doc.buffer();
        let Some(cur) = buf.tag_table().lookup("match-current") else { return };
        let matches = self.matches.borrow();
        if let Some((s, e)) = matches.get(self.match_index.get().max(0) as usize).filter(|_| self.match_index.get() >= 0) {
            buf.remove_tag(&cur, &buf.iter_at_offset(*s), &buf.iter_at_offset(*e));
        }
        self.match_index.set(idx);
        let (s, e) = matches[idx as usize];
        let mut si = buf.iter_at_offset(s);
        buf.apply_tag(&cur, &si, &buf.iter_at_offset(e));
        self.doc.scroll_to_iter(&mut si, 0.15, true, 0.0, 0.35);
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
        let buf = self.doc.buffer();
        match buf.selection_bounds() {
            Some(range) => {
                let sel = self.doc.count_words(Some(range));
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

    // -------------------------------------------------------------- zoom

    fn step_zoom(&self, d: i32) {
        let i = ZOOM_STEPS.iter().enumerate().min_by_key(|(_, z)| (**z - self.zoom.get()).abs()).map(|(i, _)| i).unwrap_or(5) as i32;
        let next = (i + d).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize;
        self.set_zoom(ZOOM_STEPS[next]);
    }

    fn set_zoom(&self, z: i32) {
        self.zoom.set(z);
        self.apply_zoom();
        if z != 100 {
            self.toast(&fill(&tr("Zoom {percent}%"), &[("percent", &z.to_string())]), 1);
        }
    }

    fn apply_zoom(&self) {
        for c in self.doc.css_classes() {
            if c.starts_with("zoom-") {
                self.doc.remove_css_class(&c);
            }
        }
        self.doc.add_css_class(&format!("zoom-{}", self.zoom.get()));
    }

    // -------------------------------------------------------------- misc

    #[cfg(debug_assertions)]
    pub fn debug_actions(self: &Rc<Self>) {
        if let Some(n) = std::env::var("AMOXTLI_SNAPSHOT_HEADING").ok().and_then(|n| n.parse::<usize>().ok()) {
            if let Some(h) = self.doc.headings().get(n) {
                self.scroll_to_heading(h);
            }
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

    fn show_side(&self, name: &str) {
        self.side_toggles.set_active_name(Some(name));
    }

    pub fn toast(&self, text: &str, timeout: u32) {
        self.toasts.add_toast(adw::Toast::builder().title(text).timeout(timeout).build());
    }

    pub fn refresh_banner(&self) {
        let st = State::load();
        let installed = config::desktop_app_info().is_some();
        let is_default = gio::AppInfo::default_for_type("text/markdown", false)
            .and_then(|a| a.id())
            .is_some_and(|id| id == format!("{APP_ID}.desktop"));
        self.banner.set_revealed(installed && !is_default && !st.banner_dismissed);
    }

    fn edit_externally(&self) {
        let Some(cur) = self.current.borrow().clone() else { return };
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
        st.zoom = self.zoom.get();
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
        let me = self.window.clone();
        WINDOWS.with(|w| w.borrow_mut().retain(|x| x.window != me));
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
    section(&[(tr("New Window"), "app.new-window"), (tr("Reload"), "win.reload")]);
    section(&[(tr("Zoom In"), "win.zoom-in"), (tr("Zoom Out"), "win.zoom-out"), (tr("Reset Zoom"), "win.zoom-reset")]);
    section(&[
        (tr("Edit in Text Editor"), "win.edit"),
        (tr("Show in Files"), "win.show-in-files"),
        (tr("Copy File Path"), "win.copy-path"),
    ]);
    section(&[
        (tr("Set as Default Markdown Viewer"), "app.make-default"),
        (tr("Keyboard Shortcuts"), "app.shortcuts"),
        (tr("About Amoxtli"), "app.about"),
    ]);
    menu
}
