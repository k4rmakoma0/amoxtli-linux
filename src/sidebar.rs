//! Sidebar panels: document outline (index) and folder browser.

use crate::document::Heading;
use crate::i18n::tr;
use adw::prelude::*;
use gtk::{gio, glib, pango};
use std::cell::RefCell;
use std::rc::Rc;

pub const MARKDOWN_SUFFIXES: &[&str] =
    &[".md", ".markdown", ".mdown", ".mkd", ".mkdn", ".mdwn", ".mdx", ".mdtext", ".rmd"];
const SKIP_DIRS: &[&str] = &["node_modules", "__pycache__", "target", "build", "dist", "venv"];

pub fn is_markdown_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    MARKDOWN_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

// =================================================================== outline

pub struct OutlinePanel {
    pub widget: gtk::Stack,
    listbox: gtk::ListBox,
    headings: RefCell<Vec<Heading>>,
    current: RefCell<Option<String>>, // slug
    on_activate: RefCell<Option<Rc<dyn Fn(Heading)>>>,
}

impl OutlinePanel {
    pub fn new() -> Rc<Self> {
        let listbox = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&listbox)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("view-list-bullet-symbolic")
            .title(tr("No Headings"))
            .description(tr("This document has no table of contents"))
            .css_classes(["compact"])
            .build();
        let widget = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        widget.add_named(&scroller, Some("list"));
        widget.add_named(&empty, Some("empty"));
        widget.set_visible_child_name("empty");

        let panel = Rc::new(Self {
            widget,
            listbox,
            headings: RefCell::new(vec![]),
            current: RefCell::new(None),
            on_activate: RefCell::new(None),
        });
        panel.listbox.connect_row_activated(glib::clone!(
            #[weak]
            panel,
            move |_, row| {
                let h = panel.headings.borrow().get(row.index() as usize).cloned();
                if let Some(h) = h {
                    panel.current.replace(Some(h.slug.clone()));
                    let cb = panel.on_activate.borrow().clone();
                    if let Some(cb) = cb {
                        cb(h);
                    }
                }
            }
        ));
        panel
    }

    pub fn connect_heading_activated<F: Fn(Heading) + 'static>(&self, f: F) {
        self.on_activate.replace(Some(Rc::new(f)));
    }

    pub fn set_headings(&self, headings: Vec<Heading>) {
        self.listbox.remove_all();
        self.current.replace(None);
        if headings.is_empty() {
            self.widget.set_visible_child_name("empty");
            self.headings.replace(headings);
            return;
        }
        let top = headings.iter().map(|h| h.level).min().unwrap_or(1);
        for h in &headings {
            let depth = (h.level - top) as i32;
            let title = if h.title.is_empty() { "—".to_owned() } else { h.title.clone() };
            let label = gtk::Label::builder()
                .label(&title)
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .margin_start(depth * 14)
                .tooltip_text(&title)
                .build();
            if depth == 0 {
                label.add_css_class("heading");
            } else if depth >= 2 {
                label.add_css_class("dim-label");
            }
            self.listbox.append(&gtk::ListBoxRow::builder().child(&label).build());
        }
        self.headings.replace(headings);
        self.widget.set_visible_child_name("list");
    }

    pub fn highlight(&self, heading: Option<&Heading>) {
        let slug = heading.map(|h| h.slug.clone());
        if *self.current.borrow() == slug {
            return;
        }
        let index = slug.as_ref().and_then(|s| self.headings.borrow().iter().position(|h| &h.slug == s));
        self.current.replace(slug);
        match index.and_then(|i| self.listbox.row_at_index(i as i32)) {
            Some(row) => self.listbox.select_row(Some(&row)),
            None => self.listbox.unselect_all(),
        }
    }
}

// ===================================================================== files

const ATTRS: &str = "standard::name,standard::display-name,standard::type,standard::is-hidden,\
                     standard::is-backup,standard::symbolic-icon,standard::content-type";

pub struct FilesPanel {
    pub widget: gtk::Box,
    title: gtk::Label,
    stack: gtk::Stack,
    selection: gtk::SingleSelection,
    root: RefCell<Option<gio::File>>,
    on_activate: RefCell<Option<Rc<dyn Fn(gio::File)>>>,
}

impl FilesPanel {
    pub fn new() -> Rc<Self> {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::Middle)
            .margin_start(18)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .css_classes(["caption-heading", "dim-label"])
            .visible(false)
            .build();

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let icon = gtk::Image::new();
            let label = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).hexpand(true).build();
            let row = gtk::Box::builder().spacing(8).build();
            row.append(&icon);
            row.append(&label);
            let expander = gtk::TreeExpander::builder().child(&row).indent_for_icon(true).build();
            item.set_child(Some(&expander));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            let Some(info) = row.item().and_downcast::<gio::FileInfo>() else { return };
            let expander = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
            expander.set_list_row(Some(&row));
            let content = expander.child().unwrap();
            let icon = content.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = icon.next_sibling().and_downcast::<gtk::Label>().unwrap();
            if let Some(gicon) = info.symbolic_icon() {
                icon.set_from_gicon(&gicon);
            }
            label.set_label(&info.display_name());
            label.set_tooltip_text(Some(&info.display_name()));
        });

        let selection = gtk::SingleSelection::builder().autoselect(false).can_unselect(true).build();
        let listview = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .single_click_activate(true)
            .css_classes(["navigation-sidebar"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&listview)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let open_btn = gtk::Button::builder()
            .label(tr("Open Folder…"))
            .action_name("win.open-folder")
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("folder-open-symbolic")
            .title(tr("No Folder"))
            .description(tr("Open a folder to browse its Markdown files"))
            .child(&open_btn)
            .css_classes(["compact"])
            .build();
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&scroller, Some("tree"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("empty");

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&title);
        widget.append(&stack);

        let panel = Rc::new(Self {
            widget,
            title,
            stack,
            selection,
            root: RefCell::new(None),
            on_activate: RefCell::new(None),
        });
        listview.connect_activate(glib::clone!(
            #[weak]
            panel,
            move |_, pos| {
                let Some(model) = panel.selection.model() else { return };
                let Some(row) = model.item(pos).and_downcast::<gtk::TreeListRow>() else { return };
                let Some(info) = row.item().and_downcast::<gio::FileInfo>() else { return };
                if info.file_type() == gio::FileType::Directory {
                    row.set_expanded(!row.is_expanded());
                } else if let Some(file) = info.attribute_object("standard::file").and_downcast::<gio::File>() {
                    let cb = panel.on_activate.borrow().clone();
                    if let Some(cb) = cb {
                        cb(file);
                    }
                }
            }
        ));
        panel
    }

    pub fn connect_file_activated<F: Fn(gio::File) + 'static>(&self, f: F) {
        self.on_activate.replace(Some(Rc::new(f)));
    }

    /// Expand every top-level folder (used for screenshots).
    #[cfg(debug_assertions)]
    pub fn expand_top_level(&self) {
        let Some(model) = self.selection.model() else { return };
        for i in 0..model.n_items() {
            if let Some(row) = model.item(i).and_downcast::<gtk::TreeListRow>() {
                if row.is_expandable() {
                    row.set_expanded(true);
                }
            }
        }
    }

    pub fn set_root(&self, folder: &gio::File) {
        if self.root.borrow().as_ref().is_some_and(|r| r.equal(folder)) {
            return;
        }
        self.root.replace(Some(folder.clone()));
        let tree = gtk::TreeListModel::new(dir_model(folder), false, false, |obj| {
            let info = obj.downcast_ref::<gio::FileInfo>()?;
            if info.file_type() != gio::FileType::Directory {
                return None;
            }
            let file = info.attribute_object("standard::file").and_downcast::<gio::File>()?;
            Some(dir_model(&file).upcast())
        });
        self.selection.set_model(Some(&tree));
        let name = folder.basename().map(|b| b.display().to_string()).unwrap_or_else(|| folder.parse_name().to_string());
        self.title.set_label(&name);
        self.title.set_tooltip_text(Some(&folder.parse_name()));
        self.title.set_visible(true);
        self.stack.set_visible_child_name("tree");
    }
}

fn dir_model(folder: &gio::File) -> gtk::SortListModel {
    let dl = gtk::DirectoryList::new(Some(ATTRS), Some(folder));
    dl.set_monitored(true);
    let filter = gtk::CustomFilter::new(|obj| {
        let info = obj.downcast_ref::<gio::FileInfo>().unwrap();
        if info.is_hidden() || info.is_backup() {
            return false;
        }
        let name = info.name();
        let name = name.to_string_lossy();
        if info.file_type() == gio::FileType::Directory {
            return !SKIP_DIRS.contains(&name.as_ref());
        }
        is_markdown_name(&name)
    });
    let sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<gio::FileInfo>().unwrap();
        let b = b.downcast_ref::<gio::FileInfo>().unwrap();
        let da = a.file_type() == gio::FileType::Directory;
        let db = b.file_type() == gio::FileType::Directory;
        if da != db {
            return if da { gtk::Ordering::Smaller } else { gtk::Ordering::Larger };
        }
        let ka = glib::FilenameCollationKey::from(a.display_name().as_str());
        let kb = glib::FilenameCollationKey::from(b.display_name().as_str());
        ka.cmp(&kb).into()
    });
    let filtered = gtk::FilterListModel::new(Some(dl), Some(filter));
    gtk::SortListModel::new(Some(filtered), Some(sorter))
}
