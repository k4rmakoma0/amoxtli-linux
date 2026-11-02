//! Small custom widgets used inside the document view.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};

// ---------------------------------------------------------------- FitImage

mod fit_image {
    use super::*;

    #[derive(Default)]
    pub struct FitImage {
        pub paintable: RefCell<Option<gdk::Paintable>>,
        pub max_width: Cell<i32>,
        pub size: Cell<(i32, i32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FitImage {
        const NAME: &'static str = "AmoxtliFitImage";
        type Type = super::FitImage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FitImage {}

    impl WidgetImpl for FitImage {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (w, h) = self.size.get();
            let s = if orientation == gtk::Orientation::Horizontal { w } else { h };
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(p) = self.paintable.borrow().as_ref() {
                let w = self.obj().width() as f64;
                let h = self.obj().height() as f64;
                p.snapshot(snapshot, w, h);
            }
        }
    }
}

glib::wrapper! {
    /// Draws a paintable scaled down (never up) to the available width.
    pub struct FitImage(ObjectSubclass<fit_image::FitImage>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FitImage {
    pub fn new(max_width: Option<i32>) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().max_width.set(max_width.unwrap_or(0));
        obj
    }

    pub fn set_paintable(&self, paintable: gdk::Paintable) {
        self.imp().paintable.replace(Some(paintable));
        self.queue_resize();
    }

    pub fn has_paintable(&self) -> bool {
        self.imp().paintable.borrow().is_some()
    }

    pub fn fit(&self, avail: i32) {
        let imp = self.imp();
        let Some(p) = imp.paintable.borrow().clone() else { return };
        let iw = p.intrinsic_width().max(1);
        let ih = p.intrinsic_height().max(1);
        let mut w = iw.min(avail.max(16));
        if imp.max_width.get() > 0 {
            w = w.min(imp.max_width.get());
        }
        let h = ((ih as f64) * (w as f64) / (iw as f64)).round() as i32;
        if (w, h) != imp.size.get() {
            imp.size.set((w, h));
            self.queue_resize();
        }
    }
}

// ------------------------------------------------------------------ FitBox

mod fit_box {
    use super::*;

    #[derive(Default)]
    pub struct FitBox {
        pub child: RefCell<Option<gtk::Widget>>,
        pub avail: Cell<i32>,
        pub fill: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FitBox {
        const NAME: &'static str = "AmoxtliFitBox";
        type Type = super::FitBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FitBox {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl FitBox {
        pub fn child_width(&self, cmin: i32, cnat: i32) -> i32 {
            let avail = self.avail.get();
            if avail <= 0 {
                cnat
            } else if self.fill.get() {
                cmin.max(avail)
            } else {
                cmin.max(cnat.min(avail))
            }
        }
    }

    impl WidgetImpl for FitBox {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else { return (0, 0, -1, -1) };
            let (cmin, cnat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            if orientation == gtk::Orientation::Horizontal {
                let w = self.avail.get().max(cmin);
                return (w, w, -1, -1);
            }
            let (_, nat, _, _) = child.measure(gtk::Orientation::Vertical, self.child_width(cmin, cnat));
            (nat, nat, -1, -1)
        }

        fn size_allocate(&self, _width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                let (cmin, cnat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                child.allocate(self.child_width(cmin, cnat), height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// Takes the available text width; its height is measured for that width.
    ///
    /// GtkTextView sizes anchored children by their plain preferred size, which
    /// for wrapping content means "tall and narrow". This wrapper fixes that.
    pub struct FitBox(ObjectSubclass<fit_box::FitBox>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FitBox {
    pub fn new(child: &impl IsA<gtk::Widget>, fill: bool) -> Self {
        let obj: Self = glib::Object::builder().property("overflow", gtk::Overflow::Hidden).build();
        child.set_parent(&obj);
        obj.imp().child.replace(Some(child.clone().upcast()));
        obj.imp().fill.set(fill);
        obj
    }

    pub fn set_avail(&self, avail: i32) {
        if self.imp().avail.get() != avail {
            self.imp().avail.set(avail);
            self.queue_resize();
        }
    }
}
