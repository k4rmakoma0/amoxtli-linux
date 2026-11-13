//! The Adw.Application: app-wide actions, shortcuts, styling and file opening.

use crate::config::{self, State, APP_ID, MIME_TYPES, VERSION};
use crate::typography::{self, ReadingFont, TextSize};
use crate::i18n::tr;
use crate::editor::{self, EditorMode, Toggle};
use crate::window::{mode_accel, Window};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::Cell;
use std::rc::Rc;

/// Table lines and header: black on light, white on dark, stronger with high contrast.
const CSS: &str = r#"
.md-document { background-color: transparent; }
.md-rule { min-height: 1px; background: alpha(currentColor, 0.2); margin: 0; }
.md-table-frame { border-radius: 8px; border: 1px solid alpha(currentColor, 0.2); }
.md-cell { border-bottom: 1px solid alpha(currentColor, 0.2); border-right: 1px solid alpha(currentColor, 0.2); }
.md-cell.md-last-row { border-bottom: none; }
.md-cell.md-last-column { border-right: none; }
.md-th { background: alpha(currentColor, 0.05); }
.md-th > textview { font-weight: bold; }
.md-cell-text, .md-cell-text > text { background-color: transparent; }
.md-img-alt { font-size: 90%; }
@media (prefers-color-scheme: dark) {
  .md-rule { background: alpha(currentColor, 0.26); }
  .md-table-frame, .md-cell { border-color: alpha(currentColor, 0.26); }
  .md-th { background: alpha(currentColor, 0.08); }
}
@media (prefers-contrast: more) {
  .md-rule { background: alpha(currentColor, 0.5); }
  .md-table-frame, .md-cell { border-color: alpha(currentColor, 0.5); }
  .md-th { background: alpha(currentColor, 0.12); }
}
"#;

thread_local! {
    /// The typeface and size style sheet, shared by every window.
    static TYPOGRAPHY: gtk::CssProvider = gtk::CssProvider::new();
    static FONT: Cell<ReadingFont> = const { Cell::new(ReadingFont::System) };
    static SIZE: Cell<TextSize> = const { Cell::new(TextSize::STANDARD) };
    static FAMILIES: std::cell::OnceCell<Vec<String>> = const { std::cell::OnceCell::new() };
}

/// The font families installed, read once.
pub fn installed_families() -> Vec<String> {
    FAMILIES.with(|f| {
        f.get_or_init(|| gtk::Label::new(None).pango_context().list_families().iter().map(|f| f.name().to_string()).collect())
            .clone()
    })
}

pub fn current_font() -> ReadingFont {
    FONT.with(Cell::get)
}

pub fn current_size() -> TextSize {
    SIZE.with(Cell::get)
}

fn apply_typography(font: ReadingFont, size: TextSize) {
    FONT.with(|f| f.set(font));
    SIZE.with(|s| s.set(size));
    let family = font.family(&installed_families());
    TYPOGRAPHY.with(|t| t.load_from_string(&typography::css(family.as_deref(), size)));
    for w in Window::all() {
        w.typography_changed();
    }
}

fn save_typography() {
    let mut st = State::load();
    st.font = current_font().name().into();
    st.text_size = current_size();
    st.save();
}

fn update_size_actions(app: &adw::Application) {
    let size = current_size();
    let set = |name: &str, on: bool| {
        if let Some(a) = app.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(on);
        }
    };
    set("text-larger", size.larger().is_some());
    set("text-smaller", size.smaller().is_some());
    set("text-reset", size != TextSize::STANDARD);
}

fn step_size(app: &adw::Application, step: fn(TextSize) -> Option<TextSize>) {
    let Some(size) = step(current_size()) else { return };
    apply_typography(current_font(), size);
    save_typography();
    update_size_actions(app);
    if let Some(w) = active(app) {
        w.toast(&crate::i18n::fill(&tr("Text size {percent}%"), &[("percent", &size.percent().to_string())]), 1);
    }
}

/// The Aa menu: the typefaces, then the size steps. Rounded only shows when a rounded
/// family is installed.
pub fn typography_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_section(None, &font_menu());
    menu.append_section(None, &size_menu());
    menu
}

pub fn font_menu() -> gio::Menu {
    let fonts = gio::Menu::new();
    let rounded = typography::rounded_family(&installed_families()).is_some();
    for font in ReadingFont::ALL.into_iter().filter(|f| *f != ReadingFont::Rounded || rounded) {
        fonts.append(Some(&font.label()), Some(&format!("app.font::{}", font.name())));
    }
    fonts
}

pub fn size_menu() -> gio::Menu {
    let sizes = gio::Menu::new();
    sizes.append(Some(&tr("Larger Text")), Some("app.text-larger"));
    sizes.append(Some(&tr("Smaller Text")), Some("app.text-smaller"));
    sizes.append(Some(&tr("Normal Size")), Some("app.text-reset"));
    sizes
}

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    glib::set_application_name("Amoxtli");
    glib::set_prgname(Some(APP_ID));
    #[cfg(debug_assertions)]
    if crate::devtools::enabled() {
        app.set_flags(app.flags() | gio::ApplicationFlags::NON_UNIQUE);
        app.connect_startup(crate::devtools::setup);
    }

    app.connect_startup(startup);
    app.connect_activate(|app| {
        match app.active_window() {
            Some(w) => w.present(),
            None => Window::new(app).window.present(),
        }
    });
    app.connect_open(|app, files, _hint| {
        for f in files {
            open_in_window(app, f, true);
        }
    });
    app.run()
}

fn active(app: &adw::Application) -> Option<Rc<Window>> {
    app.active_window().and_then(|w| Window::for_widget(&w))
}

fn open_in_window(app: &adw::Application, f: &gio::File, reuse: bool) {
    let win = match active(app) {
        Some(w) if reuse && w.is_empty() => w,
        _ => Window::new(app),
    };
    win.open_any(f);
    win.window.present();
}

fn startup(app: &adw::Application) {
    gtk::Window::set_default_icon_name(APP_ID);
    let display = gdk::Display::default().expect("no display");
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    TYPOGRAPHY.with(|t| gtk::style_context_add_provider_for_display(&display, t, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION));
    let st = State::load();
    let font = ReadingFont::from_name(&st.font).filter(|f| f.family(&installed_families()).is_some() || *f == ReadingFont::System);
    apply_typography(font.unwrap_or(ReadingFont::System), st.text_size);
    let font_action = gio::SimpleAction::new_stateful("font", Some(glib::VariantTy::STRING), &current_font().name().to_variant());
    font_action.connect_activate(|action, param| {
        let Some(font) = param.and_then(|p| p.str()).and_then(ReadingFont::from_name) else { return };
        action.set_state(&font.name().to_variant());
        apply_typography(font, current_size());
        save_typography();
    });
    app.add_action(&font_action);
    if let Some(dir) = config::dev_icon_dir() {
        gtk::IconTheme::for_display(&display).add_search_path(dir);
    }

    let add = |name: &str, accels: &[&str], f: fn(&adw::Application)| {
        let action = gio::SimpleAction::new(name, None);
        let weak = app.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                f(&app);
            }
        });
        app.add_action(&action);
        if !accels.is_empty() {
            app.set_accels_for_action(&format!("app.{name}"), accels);
        }
    };
    add("new-window", &["<Ctrl>n"], |app| Window::new(app).window.present());
    add("quit", &["<Ctrl>q"], |app| {
        for w in app.windows() {
            w.close();
        }
    });
    add("about", &[], about);
    add("shortcuts", &["<Ctrl>question"], shortcuts);
    add("make-default", &[], make_default);
    add("text-larger", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"], |app| step_size(app, TextSize::larger));
    add("text-smaller", &["<Ctrl>minus", "<Ctrl>KP_Subtract"], |app| step_size(app, TextSize::smaller));
    add("text-reset", &["<Ctrl>0", "<Ctrl>KP_0"], |app| step_size(app, |_| Some(TextSize::STANDARD)));
    update_size_actions(app);

    let accels: &[(&str, &[&str])] = &[
        ("win.open", &["<Ctrl>o"]),
        ("win.open-folder", &["<Ctrl><Shift>o"]),
        ("win.reload", &["<Ctrl>r", "F5"]),
        ("win.find", &["<Ctrl>f"]),
        ("win.toggle-sidebar", &["F9"]),
        // Ctrl+1…7 are the paragraph styles, as on macOS
        ("win.show-contents", &["<Ctrl><Alt>1"]),
        ("win.show-files", &["<Ctrl><Alt>2"]),
        ("win.back", &["<Alt>Left", "Back"]),
        ("win.forward", &["<Alt>Right", "Forward"]),
        // Ctrl+E is inline code
        ("win.edit", &["<Ctrl><Shift>e"]),
        ("win.save", &["<Ctrl>s"]),
        ("win.copy-path", &["<Ctrl><Shift>c"]),
        ("win.close", &["<Ctrl>w"]),
        ("win.top", &["<Ctrl>Home"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
    for mode in [EditorMode::Source, EditorMode::Format, EditorMode::Split] {
        app.set_accels_for_action(&format!("win.mode::{}", mode.name()), &[mode_accel(mode)]);
    }
    for tool in Toggle::ALL {
        app.set_accels_for_action(&format!("win.{}", tool.action()), tool.accels());
    }
    for name in editor::style_names() {
        app.set_accels_for_action(&format!("win.{}::{name}", editor::STYLE_ACTION), &[&editor::style_accel(name)]);
    }
}

fn make_default(app: &adw::Application) {
    let win = active(app);
    let Some(info) = config::desktop_app_info() else {
        if let Some(w) = win {
            w.toast(&tr("Install Amoxtli first (run ./install.sh)"), 3);
        }
        return;
    };
    for mime in MIME_TYPES {
        if let Err(e) = info.set_as_default_for_type(mime) {
            if let Some(w) = &win {
                w.toast(e.message(), 3);
            }
            return;
        }
    }
    for w in Window::all() {
        w.refresh_banner();
    }
    if let Some(w) = win {
        w.toast(&tr("Amoxtli is now the default Markdown viewer"), 3);
    }
}

fn about(app: &adw::Application) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Amoxtli")
        .application_icon(APP_ID)
        .developer_name(tr("Amoxtli Contributors"))
        .version(VERSION)
        .comments(format!("{}\n{}", tr("A native Markdown viewer for GNOME and Linux."), tr("Amoxtli means “book” in Nahuatl.")))
        .license_type(gtk::License::MitX11)
        .build();
    let credits = tr("translator-credits");
    if credits != "translator-credits" {
        dialog.set_translator_credits(&credits);
    }
    dialog.present(app.active_window().as_ref());
}

fn shortcuts(app: &adw::Application) {
    let dialog = adw::ShortcutsDialog::new();
    let fixed: Vec<(String, Vec<(String, &str)>)> = vec![
        (tr("General"), vec![
            (tr("Open File"), "<Ctrl>o"),
            (tr("Open Folder"), "<Ctrl><Shift>o"),
            (tr("New Window"), "<Ctrl>n"),
            (tr("Close Window"), "<Ctrl>w"),
            (tr("Quit"), "<Ctrl>q"),
            (tr("Keyboard Shortcuts"), "<Ctrl>question"),
        ]),
        (tr("Editing"), vec![
            (tr("Save"), "<Ctrl>s"),
            (tr("Undo"), "<Ctrl>z"),
            (tr("Redo"), "<Ctrl><Shift>z"),
            (tr("Show Source"), mode_accel(EditorMode::Source)),
            (tr("Show Formatted"), mode_accel(EditorMode::Format)),
            (tr("Show Split"), mode_accel(EditorMode::Split)),
        ]),
        (tr("Document"), vec![
            (tr("Find"), "<Ctrl>f"),
            (tr("Reload"), "<Ctrl>r"),
            (tr("Edit in Text Editor"), "<Ctrl><Shift>e"),
            (tr("Copy File Path"), "<Ctrl><Shift>c"),
            (tr("Back"), "<Alt>Left"),
            (tr("Forward"), "<Alt>Right"),
            (tr("Scroll to Top"), "<Ctrl>Home"),
        ]),
        (tr("View"), vec![
            (tr("Toggle Sidebar"), "F9"),
            (tr("Show Contents"), "<Ctrl><Alt>1"),
            (tr("Show Files"), "<Ctrl><Alt>2"),
            (tr("Larger Text"), "<Ctrl>plus"),
            (tr("Smaller Text"), "<Ctrl>minus"),
            (tr("Normal Size"), "<Ctrl>0"),
        ]),
    ];
    let mut groups: Vec<(String, Vec<(String, String)>)> = fixed
        .into_iter()
        .map(|(title, items)| (title, items.into_iter().map(|(n, a)| (n, a.to_owned())).collect()))
        .collect();
    let mut format = vec![];
    for name in editor::style_names() {
        format.push((editor::style_label(name), editor::style_accel(name)));
    }
    for tool in Toggle::ALL {
        format.push((tool.label().trim_end_matches('…').to_owned(), tool.accels()[0].to_owned()));
    }
    groups.push((tr("Format"), format));
    for (title, items) in groups {
        let section = adw::ShortcutsSection::new(Some(&title));
        for (name, accel) in items {
            section.add(adw::ShortcutsItem::new(&name, &accel));
        }
        dialog.add(section);
    }
    dialog.present(app.active_window().as_ref());
}
