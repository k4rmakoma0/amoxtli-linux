//! Constants and small persistent UI state (window size, typography…) stored with GKeyFile.

use crate::typography::TextSize;
use gtk::glib;
use std::path::PathBuf;

pub const APP_ID: &str = "dev.alananaya.Amoxtli";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MIME_TYPES: &[&str] = &["text/markdown", "text/x-markdown"];
pub const GETTEXT_DOMAIN: &str = "amoxtli";

/// Running inside a Flatpak sandbox.
pub fn is_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Packaged apps use portals instead of changing host associations or looking
/// up host executables. Match our snap name so an inherited environment from a
/// different snapped application doesn't affect native development builds.
pub fn is_sandboxed() -> bool {
    is_flatpak() || std::env::var("SNAP_NAME").is_ok_and(|name| name == "amoxtli")
}

/// The install prefix when running an installed binary (`<prefix>/bin/amoxtli`).
fn install_prefix() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(PathBuf::from)).flatten()
}

/// Where compiled translations live: `<prefix>/share/locale` for an installed
/// binary (any prefix), `target/locale` when running from a source checkout.
/// `AMOXTLI_LOCALEDIR` at build time overrides both (for distro packaging).
pub fn locale_dir() -> PathBuf {
    if let Some(dir) = option_env!("AMOXTLI_LOCALEDIR") {
        return PathBuf::from(dir);
    }
    match install_prefix() {
        Some(prefix) => prefix.join("share").join("locale"),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/locale")),
    }
}

/// Icons for an uninstalled (development) build.
pub fn dev_icon_dir() -> Option<PathBuf> {
    if install_prefix().is_some() {
        return None;
    }
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
    dir.is_dir().then_some(dir)
}

/// The installed .desktop entry, or None when running uninstalled.
pub fn desktop_app_info() -> Option<gtk::gio::AppInfo> {
    use gtk::gio::prelude::*;
    let id = format!("{APP_ID}.desktop");
    gtk::gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id))
}

#[derive(Debug, Clone)]
pub struct State {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// `system`, `serif`, `monospace` or `rounded`.
    pub font: String,
    pub text_size: TextSize,
    pub sidebar: bool,
    pub banner_dismissed: bool,
    /// The editor mode of the last window closed: `source`, `format` or `split`.
    pub mode: String,
}

impl Default for State {
    fn default() -> Self {
        Self { width: 1100, height: 780, maximized: false, font: "system".into(), text_size: TextSize::STANDARD, sidebar: true, banner_dismissed: false, mode: "format".into() }
    }
}

const GROUP: &str = "state";

fn state_path() -> PathBuf {
    glib::user_config_dir().join("amoxtli").join("state.ini")
}

impl State {
    pub fn load() -> Self {
        let mut st = State::default();
        let kf = glib::KeyFile::new();
        if kf.load_from_file(state_path(), glib::KeyFileFlags::NONE).is_err() {
            return st;
        }
        if let Ok(v) = kf.integer(GROUP, "width") { st.width = v.max(360); }
        if let Ok(v) = kf.integer(GROUP, "height") { st.height = v.max(300); }
        if let Ok(v) = kf.boolean(GROUP, "maximized") { st.maximized = v; }
        // 1.x saved a zoom percentage instead of a text size
        if let Ok(v) = kf.integer(GROUP, "zoom") { st.text_size = TextSize::from_zoom(v); }
        if let Ok(v) = kf.integer(GROUP, "text_size") { st.text_size = TextSize::nearest(v); }
        if let Ok(v) = kf.string(GROUP, "font") { st.font = v.into(); }
        if let Ok(v) = kf.boolean(GROUP, "sidebar") { st.sidebar = v; }
        if let Ok(v) = kf.boolean(GROUP, "banner_dismissed") { st.banner_dismissed = v; }
        if let Ok(v) = kf.string(GROUP, "mode") { st.mode = v.into(); }
        st
    }

    pub fn save(&self) {
        let kf = glib::KeyFile::new();
        kf.set_integer(GROUP, "width", self.width);
        kf.set_integer(GROUP, "height", self.height);
        kf.set_boolean(GROUP, "maximized", self.maximized);
        kf.set_integer(GROUP, "text_size", self.text_size.points());
        kf.set_string(GROUP, "font", &self.font);
        kf.set_boolean(GROUP, "sidebar", self.sidebar);
        kf.set_boolean(GROUP, "banner_dismissed", self.banner_dismissed);
        kf.set_string(GROUP, "mode", &self.mode);
        let path = state_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = kf.save_to_file(path);
    }
}
