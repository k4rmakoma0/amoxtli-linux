//! gettext setup and helpers. Message ids use `{name}` placeholders.

use crate::config;
use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};
use std::ffi::CStr;

pub fn init() {
    // SAFETY: called once at startup, before any other thread exists.
    unsafe {
        setlocale(LocaleCategory::LcAll, "");
    }
    let dir = config::locale_dir();
    let _ = bindtextdomain(config::GETTEXT_DOMAIN, dir);
    let _ = bind_textdomain_codeset(config::GETTEXT_DOMAIN, "UTF-8");
    let _ = textdomain(config::GETTEXT_DOMAIN);
}

/// Translate a message.
pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}

/// Translate a message with a plural form.
pub fn trn(singular: &str, plural: &str, n: u64) -> String {
    gettextrs::ngettext(singular, plural, n.min(u32::MAX as u64) as u32)
}

/// Substitute `{key}` placeholders.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Format an integer with the locale's thousands separator (e.g. 1,736 / 1.736).
pub fn group_digits(n: u64) -> String {
    // SAFETY: localeconv returns a pointer to static, NUL-terminated data.
    let sep = unsafe {
        let lc = libc::localeconv();
        if lc.is_null() || (*lc).thousands_sep.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*lc).thousands_sep).to_string_lossy().into_owned()
        }
    };
    let digits = n.to_string();
    if sep.is_empty() {
        return digits;
    }
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(&sep);
        }
        out.push(ch);
    }
    out
}
