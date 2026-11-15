//! The depth chart's session ([`DepthChartSession`]): a feed lease per
//! book its settings draw, the window of prices it looks at, and the frame
//! its view shows (the widget's data, described in the parent module).

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use super::{
    DepthChartSettings,
    frame::{Config, Frame, MAX_ZOOM, MIN_ZOOM},
};
use crate::{
    data::{
        SessionScope,
        ferris::{Ferris, OrderBook},
    },
    widgets::common::{Shared, books::BookFeeds},
};

/// A fitted window is kept while the books are loaded through at least
/// this much of it, and through no more than [`REFIT_OVER`] times it.
const REFIT_UNDER: f64 = 0.5;
const REFIT_OVER: f64 = 2.0;

/// The window of prices the plot shows, from one frame to the next (the
/// old chart's `zoomLevel`).
///
/// - Each frame is centered on the price between the highest bid and the
///   lowest ask of its books; one without a price stays where the last
///   one with prices was.
/// - How far it goes either side of its center is a share of the center
///   price, kept from frame to frame, so the curves do not stretch and
///   shrink under the user as levels come and go (as the order book's
///   ladder keeps its step):
///   - the first frame whose books have all arrived fits it: as far as
///     every side is loaded. Until then each frame fits what it has;
///   - a zoom makes it what the user asked for;
///   - a fitted one that the books have outgrown or no longer fill (they
///     are loaded through more than twice it, or less than half) is fitted
///     again; one the user zoomed to never is.
/// - Fitting it again ([`Self::fit`]), which drawing other books or other
///   levels of them does too ([`Config::refitted_by`]), lets go of both.
#[derive(Debug, Default)]
pub(super) struct Window {
    /// Half the window over its center price, once the user zoomed.
    zoom: Option<f64>,
    /// The same as the books fitted it, while the user has not zoomed.
    fitted: Option<f64>,
    /// The last frame's center, while that frame had prices.
    held: Option<f64>,
}

impl Window {
    /// Lets go of how wide it is: the next frame fits its books again.
    pub fn fit(&mut self) {
        self.zoom = None;
        self.fitted = None;
    }

    /// The next frame, of `books` whose topics were refused or not as
    /// `failed` says.
    pub fn frame(
        &mut self,
        config: &Rc<Config>,
        books: &[Option<Rc<OrderBook>>],
        failed: Vec<bool>,
        width: f64,
        height: f64,
    ) -> Frame {
        let held = self.held;
        let build =
            |share: Option<f64>| Frame::build(config.clone(), books, width, height, share, held);
        let mut frame = build(self.zoom.or(self.fitted));
        if frame.has_prices {
            self.held = Some(frame.center);
        }
        if self.zoom.is_none() && frame.center > 0.0 && frame.settled(&failed) {
            let fit = frame.fit / frame.center;
            let kept = self
                .fitted
                .is_some_and(|kept| fit >= kept * REFIT_UNDER && fit <= kept * REFIT_OVER);
            // The first fit is the frame's own already; a later one is not.
            if !kept && self.fitted.replace(fit).is_some() {
                frame = build(self.fitted);
            }
        }
        frame.failed = failed;
        frame
    }

    /// Makes `frame`'s window `factor` times as wide, within what a zoom
    /// may be. Says whether there was a window to zoom: a frame without
    /// prices has none.
    pub fn zoom(&mut self, frame: &Frame, factor: f64) -> bool {
        if !(frame.has_prices && frame.center > 0.0 && factor.is_finite() && factor > 0.0) {
            return false;
        }
        self.zoom = Some((frame.span / frame.center * factor).clamp(MIN_ZOOM, MAX_ZOOM));
        true
    }
}

/// A depth chart's session state: the window it looks at, and what its
/// view shows (its feeds and settings live in the effects `new` starts).
pub(crate) struct DepthChartSession {
    /// What the view shows, updated while the widget is visible.
    pub(super) frame: Signal<Shared<Frame>>,
    /// The widget's size below its top bar, in points, which the view
    /// measures.
    pub(super) size: Signal<(f32, f32)>,
    window: Rc<RefCell<Window>>,
    /// Bumped when the user moved the window: the frame is made at once.
    moved: Signal<u64>,
}

impl DepthChartSession {
    /// A depth chart following `settings`, the session's copy of the
    /// tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<DepthChartSettings>,
    ) -> Self {
        let frame = signal(Shared::new(Frame::empty()));
        let size = signal((1.0_f32, 1.0_f32));
        let moved = signal(0_u64);
        let window: Rc<RefCell<Window>> = Rc::default();
        // A lease per book drawn; dirty when one brought something (or the
        // settings changed) since the frame was made.
        let feeds = BookFeeds::new(ferris);

        let config = computed(move || settings.with(|settings| Rc::new(Config::of(settings))));
        // The feeds follow the books and the depth the settings draw, and
        // the window is fitted again when they draw other books or levels.
        {
            let (feeds, window) = (feeds.clone(), window.clone());
            let drawn = RefCell::new(None::<Rc<Config>>);
            effect(move || {
                let next = config.get();
                untracked(|| {
                    feeds.follow(&next.layers, next.depth);
                    let refitted = drawn
                        .borrow()
                        .as_ref()
                        .is_some_and(|before| before.refitted_by(&next));
                    if refitted {
                        window.borrow_mut().fit();
                    }
                    *drawn.borrow_mut() = Some(next);
                });
                feeds.mark();
            });
        }

        {
            let window = window.clone();
            scope.while_visible(move || {
                let refresh: Rc<dyn Fn()> = {
                    let (feeds, window) = (feeds.clone(), window.clone());
                    Rc::new(move || {
                        let config = config.get_untracked();
                        let (books, failed) = feeds.take();
                        let (width, height) = size.get_untracked();
                        let next = window.borrow_mut().frame(
                            &config,
                            &books,
                            failed,
                            f64::from(width),
                            f64::from(height),
                        );
                        frame.try_set(Shared::new(next));
                    })
                };
                // New books and settings: at most once a frame.
                feeds.each_frame(refresh.clone());
                // A resize or a move of the window: at once.
                effect(move || {
                    size.get();
                    moved.get();
                    untracked(|| refresh());
                });
            });
        }

        Self {
            frame,
            size,
            window,
            moved,
        }
    }

    /// Makes the window `factor` times as wide about its center, as the
    /// wheel asks.
    pub(super) fn zoom(&self, factor: f64) {
        let frame = self.frame.get_untracked();
        if self.window.borrow_mut().zoom(&frame.0, factor) {
            self.moved.update(|moves| *moves = moves.wrapping_add(1));
        }
    }

    /// Fits the window to the books again, as a double click on the plot
    /// asks.
    pub(super) fn fit(&self) {
        self.window.borrow_mut().fit();
        self.moved.update(|moves| *moves = moves.wrapping_add(1));
    }
}
