//! Amoxtli — a native Markdown editor for Linux (GTK 4 + libadwaita).

mod application;
mod config;
#[cfg(debug_assertions)]
mod devtools;
mod document;
mod editor;
mod i18n;
mod markdown;
mod sidebar;
mod typography;
mod widgets;
mod window;

fn main() -> gtk::glib::ExitCode {
    i18n::init();
    application::run()
}
