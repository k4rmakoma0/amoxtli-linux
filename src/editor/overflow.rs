//! A row of tools that moves them into an overflow menu one at a time, from the right, as
//! the window narrows. A group moved as a whole would leave its whole width empty instead.

use crate::i18n::tr;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

const SPACING: i32 = 4;

pub struct Item {
    pub widget: gtk::Widget,
    /// How the item shows in the overflow menu; `None` for separators.
    pub entry: Option<Entry>,
}

pub enum Entry {
    Item(gio::MenuItem),
    Submenu(String, gio::MenuModel),
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct OverflowBar {
        pub items: RefCell<Vec<Item>>,
        pub more: RefCell<Option<gtk::MenuButton>>,
        /// How many items are in the overflow menu.
        pub hidden: Cell<usize>,
        pub menu_hidden: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OverflowBar {
        const NAME: &'static str = "AmoxtliOverflowBar";
        type Type = super::OverflowBar;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for OverflowBar {
        fn constructed(&self) {
            self.parent_constructed();
            let more = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .tooltip_text(tr("More Formatting"))
                .css_classes(["flat"])
                .valign(gtk::Align::Center)
                .build();
            more.set_parent(&*self.obj());
            more.set_child_visible(false);
            self.more.replace(Some(more));
            self.menu_hidden.set(usize::MAX);
        }

        fn dispose(&self) {
            for item in self.items.borrow_mut().drain(..) {
                item.widget.unparent();
            }
            if let Some(more) = self.more.take() {
                more.unparent();
            }
        }
    }

    impl WidgetImpl for OverflowBar {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let more = self.more.borrow().clone().unwrap();
            let items = self.items.borrow();
            if orientation == gtk::Orientation::Horizontal {
                let (more_min, _, _, _) = more.measure(orientation, -1);
                let natural: i32 = items.iter().map(|i| i.widget.measure(orientation, -1).1).sum::<i32>()
                    + SPACING * items.len().saturating_sub(1) as i32;
                (more_min, natural.max(more_min), -1, -1)
            } else {
                let (min, nat) = items.iter().map(|i| &i.widget).chain(std::iter::once(more.upcast_ref()))
                    .map(|w| w.measure(orientation, -1))
                    .fold((0, 0), |(a, b), (m, n, _, _)| (a.max(m), b.max(n)));
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let more = self.more.borrow().clone().unwrap();
            let items = self.items.borrow();
            let widths: Vec<i32> = items.iter().map(|i| i.widget.measure(gtk::Orientation::Horizontal, -1).1).collect();
            let total: i32 = widths.iter().sum::<i32>() + SPACING * widths.len().saturating_sub(1) as i32;
            let shown = if total <= width {
                items.len()
            } else {
                let more_width = more.measure(gtk::Orientation::Horizontal, -1).1;
                let room = width - more_width - SPACING;
                let mut used = 0;
                let mut count = 0;
                for w in &widths {
                    let next = used + w + if count > 0 { SPACING } else { 0 };
                    if next > room {
                        break;
                    }
                    used = next;
                    count += 1;
                }
                // a separator at the end of the row leads nowhere
                while count > 0 && items[count - 1].entry.is_none() {
                    count -= 1;
                }
                count
            };
            let mut x = 0;
            for (i, item) in items.iter().enumerate() {
                let visible = i < shown;
                item.widget.set_child_visible(visible);
                if visible {
                    item.widget.size_allocate(&gtk::Allocation::new(x, 0, widths[i], height), -1);
                    x += widths[i] + SPACING;
                }
            }
            let overflowing = shown < items.len();
            more.set_child_visible(overflowing);
            if overflowing {
                let w = more.measure(gtk::Orientation::Horizontal, -1).1;
                let at = if shown == 0 { 0 } else { x };
                more.size_allocate(&gtk::Allocation::new(at.min(width - w).max(0), 0, w, height), -1);
            }
            let hidden = items.len() - shown;
            self.hidden.set(hidden);
            if hidden != self.menu_hidden.get() {
                // Changing the menu while allocating would queue another allocation.
                let obj = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(obj) = obj.upgrade() {
                        obj.rebuild_menu();
                    }
                });
            }
        }
    }
}

glib::wrapper! {
    pub struct OverflowBar(ObjectSubclass<imp::OverflowBar>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for OverflowBar {
    fn default() -> Self {
        Self::new()
    }
}

impl OverflowBar {
    pub fn new() -> Self {
        let bar: Self = glib::Object::new();
        bar.set_accessible_role(gtk::AccessibleRole::Toolbar);
        bar
    }

    pub fn push(&self, widget: &impl IsA<gtk::Widget>, entry: Entry) {
        self.add(Item { widget: widget.clone().upcast(), entry: Some(entry) });
    }

    pub fn push_separator(&self) {
        let sep = gtk::Separator::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(4)
            .margin_end(4)
            .build();
        self.add(Item { widget: sep.upcast(), entry: None });
    }

    fn add(&self, item: Item) {
        let more = self.imp().more.borrow().clone().unwrap();
        item.widget.insert_before(self, Some(&more));
        self.imp().items.borrow_mut().push(item);
        self.queue_resize();
    }

    fn rebuild_menu(&self) {
        let imp = self.imp();
        let hidden = imp.hidden.get();
        if hidden == imp.menu_hidden.get() {
            return;
        }
        imp.menu_hidden.set(hidden);
        let menu = gio::Menu::new();
        let mut section = gio::Menu::new();
        let items = imp.items.borrow();
        for item in &items[items.len() - hidden..] {
            match &item.entry {
                None => {
                    if section.n_items() > 0 {
                        menu.append_section(None, &section);
                        section = gio::Menu::new();
                    }
                }
                Some(Entry::Item(i)) => section.append_item(i),
                Some(Entry::Submenu(label, model)) => section.append_submenu(Some(label), model),
            }
        }
        if section.n_items() > 0 {
            menu.append_section(None, &section);
        }
        if let Some(more) = imp.more.borrow().as_ref() {
            more.set_menu_model(Some(&menu));
        }
    }
}
