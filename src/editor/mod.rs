//! The editors and their formatting tools.

pub mod dialogs;
pub mod overflow;
pub mod source_view;

use crate::i18n::{fill, tr};
use crate::markdown::active::{ActiveFormats, ListKind};
use crate::markdown::format::FormatCommand;
use gtk::gio;
use gtk::prelude::*;

/// How a window shows the document: as Markdown source, formatted, or both side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorMode {
    Source,
    Format,
    Split,
}

impl EditorMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Format => "format",
            Self::Split => "split",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "source" => Some(Self::Source),
            "format" => Some(Self::Format),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    pub fn shows_source(self) -> bool {
        self != Self::Format
    }

    pub fn shows_format(self) -> bool {
        self != Self::Source
    }
}

/// A formatting tool with an on/off state, as a boolean stateful `win.` action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    InlineCode,
    BulletList,
    NumberedList,
    Quote,
    Link,
    CodeBlock,
}

impl Toggle {
    pub const ALL: [Toggle; 10] = [
        Toggle::Bold,
        Toggle::Italic,
        Toggle::Underline,
        Toggle::Strikethrough,
        Toggle::InlineCode,
        Toggle::BulletList,
        Toggle::NumberedList,
        Toggle::Quote,
        Toggle::Link,
        Toggle::CodeBlock,
    ];

    pub fn action(self) -> &'static str {
        match self {
            Self::Bold => "format-bold",
            Self::Italic => "format-italic",
            Self::Underline => "format-underline",
            Self::Strikethrough => "format-strikethrough",
            Self::InlineCode => "format-code",
            Self::BulletList => "format-bullets",
            Self::NumberedList => "format-numbers",
            Self::Quote => "format-quote",
            Self::Link => "format-link",
            Self::CodeBlock => "format-code-block",
        }
    }

    pub fn label(self) -> String {
        tr(match self {
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Underline => "Underline",
            Self::Strikethrough => "Strikethrough",
            Self::InlineCode => "Code",
            Self::BulletList => "Bulleted List",
            Self::NumberedList => "Numbered List",
            Self::Quote => "Quote",
            Self::Link => "Link…",
            Self::CodeBlock => "Code Block",
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Bold => "format-text-bold-symbolic",
            Self::Italic => "format-text-italic-symbolic",
            Self::Underline => "format-text-underline-symbolic",
            Self::Strikethrough => "format-text-strikethrough-symbolic",
            Self::InlineCode => "amoxtli-code-symbolic",
            Self::BulletList => "view-list-bullet-symbolic",
            Self::NumberedList => "view-list-ordered-symbolic",
            Self::Quote => "amoxtli-quote-symbolic",
            Self::Link => "insert-link-symbolic",
            Self::CodeBlock => "amoxtli-code-block-symbolic",
        }
    }

    pub fn accels(self) -> &'static [&'static str] {
        match self {
            Self::Bold => &["<Ctrl>b"],
            Self::Italic => &["<Ctrl>i"],
            Self::Underline => &["<Ctrl>u"],
            Self::Strikethrough => &["<Ctrl><Shift>x"],
            Self::InlineCode => &["<Ctrl>e"],
            Self::BulletList => &["<Ctrl><Shift>8"],
            Self::NumberedList => &["<Ctrl><Shift>7"],
            Self::Quote => &["<Ctrl>apostrophe"],
            Self::Link => &["<Ctrl>k"],
            Self::CodeBlock => &["<Ctrl><Alt>c"],
        }
    }

    /// `None` for the tools that ask for something first.
    pub fn command(self) -> Option<FormatCommand> {
        Some(match self {
            Self::Bold => FormatCommand::Bold,
            Self::Italic => FormatCommand::Italic,
            Self::Underline => FormatCommand::Underline,
            Self::Strikethrough => FormatCommand::Strikethrough,
            Self::InlineCode => FormatCommand::InlineCode,
            Self::BulletList => FormatCommand::BulletList,
            Self::NumberedList => FormatCommand::NumberedList,
            Self::Quote => FormatCommand::Quote,
            Self::Link => return None,
            Self::CodeBlock => FormatCommand::CodeBlock,
        })
    }

    pub fn is_active(self, f: &ActiveFormats) -> bool {
        match self {
            Self::Bold => f.strong,
            Self::Italic => f.emphasis,
            Self::Underline => f.underline,
            Self::Strikethrough => f.strikethrough,
            Self::InlineCode => f.code,
            Self::BulletList => f.list == Some(ListKind::Bulleted),
            Self::NumberedList => f.list == Some(ListKind::Numbered),
            Self::Quote => f.quote,
            Self::Link => f.link,
            Self::CodeBlock => f.code_block,
        }
    }
}

/// Tools that only insert something, so they are never on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    Table,
    HorizontalRule,
}

impl Insert {
    pub fn action(self) -> &'static str {
        match self {
            Self::Table => "format-table",
            Self::HorizontalRule => "format-rule",
        }
    }

    pub fn label(self) -> String {
        tr(match self {
            Self::Table => "Table…",
            Self::HorizontalRule => "Horizontal Line",
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Table => "amoxtli-table-symbolic",
            Self::HorizontalRule => "amoxtli-rule-symbolic",
        }
    }
}

pub const STYLE_ACTION: &str = "paragraph-style";

/// The paragraph styles, as `win.paragraph-style` targets: `body`, `h1`…`h6`.
pub fn style_names() -> [&'static str; 7] {
    ["body", "h1", "h2", "h3", "h4", "h5", "h6"]
}

pub fn style_label(name: &str) -> String {
    match name.strip_prefix('h').and_then(|n| n.parse::<u8>().ok()) {
        Some(level) => fill(&tr("Heading {n}"), &[("n", &level.to_string())]),
        None => tr("Text"),
    }
}

pub fn style_level(name: &str) -> u8 {
    name.strip_prefix('h').and_then(|n| n.parse().ok()).unwrap_or(0)
}

pub fn style_name(formats: &ActiveFormats) -> &'static str {
    style_names().get(formats.heading as usize).copied().unwrap_or("body")
}

/// `<Ctrl>1`…`<Ctrl>6` for headings and `<Ctrl>7` for body text, as in the macOS app.
pub fn style_accel(name: &str) -> String {
    match style_level(name) {
        0 => "<Ctrl>7".into(),
        n => format!("<Ctrl>{n}"),
    }
}

fn item(label: &str, action: &str, icon: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(&format!("win.{action}")));
    if let Some(icon) = icon {
        item.set_attribute_value("verb-icon", Some(&icon.to_variant()));
    }
    item
}

pub fn style_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for name in style_names() {
        menu.append(Some(&style_label(name)), Some(&format!("win.{STYLE_ACTION}::{name}")));
    }
    menu
}

pub fn toggle_item(tool: Toggle) -> gio::MenuItem {
    item(&tool.label(), tool.action(), None)
}

pub fn insert_item(tool: Insert) -> gio::MenuItem {
    item(&tool.label(), tool.action(), None)
}

/// The formatting tools grouped as the menus list them.
pub fn format_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_section(None, &style_menu());
    let section = |tools: &[Toggle], inserts: &[Insert]| {
        let s = gio::Menu::new();
        for &t in tools {
            s.append_item(&toggle_item(t));
        }
        for &i in inserts {
            s.append_item(&insert_item(i));
        }
        menu.append_section(None, &s);
    };
    use Toggle::*;
    section(&[Bold, Italic, Underline, Strikethrough, InlineCode], &[]);
    section(&[BulletList, NumberedList, Quote], &[]);
    section(&[Link, CodeBlock], &[Insert::Table, Insert::HorizontalRule]);
    menu
}

/// The text views' context menu: GTK's own items (cut, copy, paste, select all…), then
/// the Format submenu.
pub fn context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_submenu(Some(&tr("Format")), &format_menu());
    menu
}

/// A shortcut as people read it, as `Ctrl+B`.
pub fn accel_label(accel: &str) -> Option<String> {
    gtk::accelerator_parse(accel).map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string())
}

fn tooltip(label: &str, accel: Option<&str>) -> String {
    let label = label.trim_end_matches('…');
    match accel.and_then(accel_label) {
        Some(keys) => format!("{label} ({keys})"),
        None => label.to_owned(),
    }
}

/// The formatting toolbar: Style, then the inline styles, the blocks and the insertions.
pub fn format_bar() -> overflow::OverflowBar {
    use overflow::Entry;
    let bar = overflow::OverflowBar::new();
    let style = gtk::MenuButton::builder()
        .icon_name("amoxtli-paragraph-symbolic")
        .menu_model(&style_menu())
        .tooltip_text(tr("Style"))
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build();
    bar.push(&style, Entry::Submenu(tr("Style"), style_menu().upcast()));
    use Toggle::*;
    for group in [&[Bold, Italic, Underline, Strikethrough, InlineCode][..], &[BulletList, NumberedList, Quote], &[Link, CodeBlock]] {
        bar.push_separator();
        for &tool in group {
            let button = gtk::ToggleButton::builder()
                .icon_name(tool.icon())
                .action_name(format!("win.{}", tool.action()))
                .tooltip_text(tooltip(&tool.label(), tool.accels().first().copied()))
                .css_classes(["flat"])
                .valign(gtk::Align::Center)
                .build();
            button.update_property(&[gtk::accessible::Property::Label(tool.label().trim_end_matches('…'))]);
            bar.push(&button, Entry::Item(toggle_item(tool)));
        }
    }
    for tool in [Insert::Table, Insert::HorizontalRule] {
        let button = gtk::Button::builder()
            .icon_name(tool.icon())
            .action_name(format!("win.{}", tool.action()))
            .tooltip_text(tooltip(&tool.label(), None))
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(tool.label().trim_end_matches('…'))]);
        bar.push(&button, Entry::Item(insert_item(tool)));
    }
    bar
}
