//! Keeps the two editors of the split view at the same place in the document. The side the
//! person scrolls, or types in, leads; the other follows, block by block (`ScrollMap`).

use super::rich::RichView;
use super::source_view::SourceView;
use crate::markdown::scroll::{source_block_starts, ScrollMap};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Source,
    Formatted,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Source => Side::Formatted,
            Side::Formatted => Side::Source,
        }
    }
}

/// The wheel, and the momentum after it, lead for a moment after each event.
const WHEEL_LEAD: Duration = Duration::from_millis(300);
/// Once edits pause, the side not being typed in catches up.
const FOLLOW_DELAY: Duration = Duration::from_millis(250);

pub struct SplitScroll {
    source: SourceView,
    rich: RichView,
    source_scroller: gtk::ScrolledWindow,
    rich_scroller: gtk::ScrolledWindow,
    enabled: Cell<bool>,
    /// Where blocks start: characters in the source, lines in the formatted editor. Worked out
    /// again only when the text changes.
    starts: RefCell<Option<(Vec<usize>, Vec<i32>)>>,
    /// Rebuilt when the text or the layout changes.
    map: RefCell<Option<ScrollMap>>,
    following: Cell<bool>,
    wheel: Cell<Option<(Side, Instant)>>,
    /// Dragging a scrollbar leads until it ends.
    dragging: Cell<Option<Side>>,
    pending: RefCell<Option<glib::SourceId>>,
}

impl SplitScroll {
    pub fn new(source: &SourceView, rich: &RichView, source_scroller: &gtk::ScrolledWindow, rich_scroller: &gtk::ScrolledWindow) -> Rc<Self> {
        let this = Rc::new(Self {
            source: source.clone(),
            rich: rich.clone(),
            source_scroller: source_scroller.clone(),
            rich_scroller: rich_scroller.clone(),
            enabled: Cell::new(false),
            starts: RefCell::new(None),
            map: RefCell::new(None),
            following: Cell::new(false),
            wheel: Cell::new(None),
            dragging: Cell::new(None),
            pending: RefCell::new(None),
        });
        this.attach(Side::Source);
        this.attach(Side::Formatted);
        this
    }

    fn scroller(&self, side: Side) -> &gtk::ScrolledWindow {
        match side {
            Side::Source => &self.source_scroller,
            Side::Formatted => &self.rich_scroller,
        }
    }

    fn attach(self: &Rc<Self>, side: Side) {
        let scroller = self.scroller(side);
        let adj = scroller.vadjustment();
        let weak = Rc::downgrade(self);
        adj.connect_value_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                s.scrolled(side);
            }
        });
        let weak = Rc::downgrade(self);
        // upper and page size: the layout moved
        adj.connect_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                s.map.replace(None);
            }
        });
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        wheel.connect_scroll(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                s.wheel.set(Some((side, Instant::now())));
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        wheel.connect_decelerate(move |_, _, _| {
            // the momentum after a touchpad swipe keeps leading a little longer
            if let Some(s) = weak.upgrade() {
                s.wheel.set(Some((side, Instant::now() + Duration::from_millis(1200))));
            }
        });
        scroller.add_controller(wheel);
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                s.dragging.set(Some(side));
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                if s.dragging.get() == Some(side) {
                    s.dragging.set(None);
                }
            }
        });
        scroller.vscrollbar().add_controller(drag);
    }

    /// Only the split view scrolls in step. Turning it on lines the other side up at once.
    pub fn set_enabled(self: &Rc<Self>, on: bool) {
        self.enabled.set(on);
        if on {
            self.content_changed();
        }
    }

    /// A side's text or typography changed, so blocks moved. Once the changes pause, the side
    /// that isn't being typed in catches up with the one that is.
    pub fn content_changed(self: &Rc<Self>) {
        self.starts.replace(None);
        self.map.replace(None);
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        self.pending.replace(Some(glib::timeout_add_local_once(FOLLOW_DELAY, move || {
            let Some(s) = weak.upgrade() else { return };
            s.pending.replace(None);
            if let Some(side) = s.leader().or(Some(Side::Source)) {
                s.follow(side);
            }
        })));
    }

    fn scrolled(&self, side: Side) {
        if self.enabled.get() && !self.following.get() && self.leader() == Some(side) {
            self.follow(side);
        }
    }

    /// The side being scrolled, or else the one with keyboard focus, whose caret may scroll
    /// it. A side that scrolls for any other reason leads nothing.
    pub fn leader(&self) -> Option<Side> {
        if let Some(side) = self.dragging.get() {
            return Some(side);
        }
        if let Some((side, at)) = self.wheel.get() {
            let now = Instant::now();
            if at > now || now - at < WHEEL_LEAD {
                return Some(side);
            }
        }
        if self.source.has_focus() {
            Some(Side::Source)
        } else if self.rich.has_focus() || self.rich.focused_cell().is_some() {
            Some(Side::Formatted)
        } else {
            None
        }
    }

    /// The other side scrolls to where the leading one is.
    pub fn follow(&self, side: Side) {
        if !self.enabled.get() {
            return;
        }
        // Measuring may lay text out and move either view; none of that is the person scrolling.
        self.following.set(true);
        let map = self.current_map();
        let offset = self.scroller(side).vadjustment().value();
        let target = match side {
            Side::Source => map.formatted_for_source(offset),
            Side::Formatted => map.source_for_formatted(offset),
        };
        let other = self.scroller(side.other()).vadjustment();
        if (other.value() - target).abs() >= 0.5 {
            other.set_value(target);
        }
        self.following.set(false);
    }

    pub fn current_map(&self) -> ScrollMap {
        if let Some(map) = self.map.borrow().clone() {
            return map;
        }
        let (source_starts, rich_lines) = self
            .starts
            .borrow_mut()
            .get_or_insert_with(|| (source_block_starts(&self.source.text()), self.rich.block_start_lines()))
            .clone();
        let end = |s: &gtk::ScrolledWindow| {
            let adj = s.vadjustment();
            (adj.upper() - adj.page_size()).max(0.0)
        };
        let map = ScrollMap::new(
            &self.source.offsets_of(&source_starts),
            &self.rich.offsets_of(&rich_lines),
            end(&self.source_scroller),
            end(&self.rich_scroller),
        );
        self.map.replace(Some(map.clone()));
        map
    }
}
