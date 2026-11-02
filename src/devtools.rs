//! Debug-build helpers for automated screenshots (not compiled into release builds).
//!
//! AMOXTLI_SNAPSHOT=out.png        save the window to a PNG after loading, then quit
//! AMOXTLI_SNAPSHOT_SIZE=WxH        window size
//! AMOXTLI_SNAPSHOT_DARK=1          force dark style
//! AMOXTLI_SNAPSHOT_HEADING=n       scroll to the n-th heading
//! AMOXTLI_SNAPSHOT_SEARCH=text     open the search bar with this text
//! AMOXTLI_SNAPSHOT_SELECT=n        select the first n characters

use adw::prelude::*;
use gtk::{glib, graphene};
use std::time::Duration;

pub fn enabled() -> bool {
    std::env::var_os("AMOXTLI_SNAPSHOT").is_some()
}

pub fn setup(app: &adw::Application) {
    if std::env::var("AMOXTLI_SNAPSHOT_DARK").is_ok() {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    }
    let app = app.clone();
    glib::timeout_add_local_once(Duration::from_millis(1200), move || {
        let Some(win) = app.active_window() else { return app.quit() };
        if let Ok(size) = std::env::var("AMOXTLI_SNAPSHOT_SIZE") {
            if let Some((w, h)) = size.split_once('x') {
                win.unmaximize();
                win.set_default_size(w.parse().unwrap_or(1100), h.parse().unwrap_or(780));
            }
        }
        if let Some(w) = crate::window::Window::for_widget(&win) {
            w.debug_actions();
        }
        let app = app.clone();
        glib::timeout_add_local_once(Duration::from_millis(900), move || {
            let path = std::env::var("AMOXTLI_SNAPSHOT").unwrap();
            let (w, h) = (win.width(), win.height());
            let paintable = gtk::WidgetPaintable::new(Some(&win));
            let snap = gtk::Snapshot::new();
            paintable.snapshot(&snap, w as f64, h as f64);
            if let (Some(node), Some(renderer)) = (snap.to_node(), win.renderer()) {
                let tex = renderer.render_texture(node, Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
                let _ = tex.save_to_png(&path);
                println!("saved {path} {w}x{h}");
            }
            app.quit();
        });
    });
}
