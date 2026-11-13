//! The Adw.Application: app-wide actions, shortcuts, styling and file opening.

use crate::config::{self, APP_ID, MIME_TYPES, VERSION};
use crate::i18n::tr;
use crate::editor::{self, EditorMode, Toggle};
use crate::window::{mode_accel, Window};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::rc::Rc;

const CSS: &str = r#"
.md-document { background-color: transparent; }
.md-rule { min-height: 1px; background: alpha(currentColor, 0.15); margin: 0; }
.md-table-frame { border-radius: 10px; border: 1px solid alpha(currentColor, 0.14); }
.md-table-frame > border { border: none; }
.md-cell { padding: 7px 14px; border-bottom: 1px solid alpha(currentColor, 0.09); }
.md-cell.md-last-row { border-bottom: none; }
.md-th { font-weight: bold; background: alpha(currentColor, 0.05); }
.md-striped { background: alpha(currentColor, 0.025); }
.md-img-alt { font-size: 90%; }
"#;

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
    let zoom_css: String = (50..=300)
        .step_by(10)
        .map(|z| format!(".md-document.zoom-{z} {{ font-size: {:.2}em; }}\n", z as f64 / 100.0))
        .collect();
    provider.load_from_string(&format!("{CSS}{zoom_css}"));
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
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

    let accels: &[(&str, &[&str])] = &[
        ("win.open", &["<Ctrl>o"]),
        ("win.open-folder", &["<Ctrl><Shift>o"]),
        ("win.reload", &["<Ctrl>r", "F5"]),
        ("win.find", &["<Ctrl>f"]),
        ("win.toggle-sidebar", &["F9"]),
        // Ctrl+1…7 are the paragraph styles, as on macOS
        ("win.show-contents", &["<Ctrl><Alt>1"]),
        ("win.show-files", &["<Ctrl><Alt>2"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
        ("win.zoom-reset", &["<Ctrl>0", "<Ctrl>KP_0"]),
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
            (tr("Zoom In"), "<Ctrl>plus"),
            (tr("Zoom Out"), "<Ctrl>minus"),
            (tr("Reset Zoom"), "<Ctrl>0"),
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
