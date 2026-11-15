//! The order book's session ([`OrderBookSession`]): a feed lease per book
//! its settings draw, where the ladder looks, and the frame its view shows
//! (the widget's data, described in the parent module).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nsh_core::prelude::*;

use super::{
    OrderBookSettings,
    frame::{Config, Frame},
    settings::{BookLayout, Layer, Pair},
};
use crate::{
    data::{
        SessionScope,
        ferris::{Ferris, OrderBook, OrderBookFeed},
    },
    widgets::common::{Keyed, Shared},
};

/// Where the ladder looks, from one frame to the next (the old renderer's
/// `centerPriceOverride` and `ladderStepOverride`).
///
/// - While a book is followed, each frame is centered on that book's mid
///   price. A followed book that has not arrived, or has one side only,
///   holds the view where it was: it never falls back on the other book.
/// - With none followed, the first frame that has prices fixes the center
///   (on the first book's mid price), and it stays until the user moves
///   it.
/// - The first frame that has prices fixes the step a row stands for too,
///   so rows do not rescale under the user as levels come and go.
/// - Laying the ladder out anew ([`Config::relaid_by`]) or asking for it
///   ([`Self::reset`]) lets both go.
#[derive(Debug, Default)]
pub(super) struct Ladder {
    center: Option<f64>,
    step: Option<f64>,
    /// The last frame's center, while that frame had prices.
    last: Option<f64>,
}

impl Ladder {
    /// Lets go of the center and the step: the next frame fits them again.
    pub fn reset(&mut self) {
        self.center = None;
        self.step = None;
    }

    /// The next frame, following `follow`'s book if it names one.
    pub fn frame(
        &mut self,
        config: &Rc<Config>,
        follow: Option<Pair>,
        books: &[Option<Rc<OrderBook>>],
        width: f64,
        height: f64,
    ) -> Frame {
        if let Some(pair) = follow {
            let mid = config
                .layers
                .iter()
                .position(|layer| layer.pair == pair)
                .and_then(|layer| books.get(layer)?.as_ref()?.midpoint());
            self.center = Some(
                mid.or(self.center)
                    .unwrap_or_else(|| self.last.unwrap_or(0.0)),
            );
        }
        let frame = Frame::build(config.clone(), books, width, height, self.center, self.step);
        if frame.has_prices && frame.step > 0.0 {
            self.center = Some(frame.center_price);
            self.step.get_or_insert(frame.step);
        }
        self.last = frame.has_prices.then_some(frame.center_price);
        frame
    }

    /// Moves the center `rows` rows up `frame`'s ladder (toward higher
    /// prices).
    pub fn pan(&mut self, frame: &Frame, rows: f64) {
        self.center = Some(frame.shifted_center(rows));
    }
}

/// An order book's session state: where its ladder looks, and what its
/// view shows (its feeds and settings live in the effects `new` starts).
pub(crate) struct OrderBookSession {
    /// What the view shows, updated while the widget is visible.
    pub(super) frame: Signal<Shared<Frame>>,
    /// The widget's size below its top bar, in points, which the view
    /// measures.
    pub(super) size: Signal<(f32, f32)>,
    ladder: Rc<RefCell<Ladder>>,
    /// The book the ladder follows: the settings', until the user moves
    /// the ladder (which stops it at once, before the settings say so).
    follow: Rc<Cell<Option<Pair>>>,
    /// Bumped when the user moved the ladder: the frame is made at once.
    moved: Signal<u64>,
}

impl OrderBookSession {
    /// An order book following `settings`, the session's copy of the
    /// tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<OrderBookSettings>,
    ) -> Self {
        let frame = signal(Shared::new(Frame::empty()));
        let size = signal((1.0_f32, 1.0_f32));
        let moved = signal(0_u64);
        let ladder: Rc<RefCell<Ladder>> = Rc::default();
        let follow: Rc<Cell<Option<Pair>>> = Rc::default();

        let feeds: Rc<Keyed<String, OrderBookFeed>> = Rc::default();
        // Books arrived (or the settings changed) since the frame was made.
        let dirty = Rc::new(Cell::new(true));
        // Bumped when `dirty` is set, so the visible view schedules a frame.
        let wake = signal(0_u64);
        let mark: Rc<dyn Fn()> = {
            let (dirty, wakes) = (dirty.clone(), Cell::new(0_u64));
            Rc::new(move || {
                if !dirty.replace(true) {
                    wakes.set(wakes.get().wrapping_add(1));
                    wake.try_set(wakes.get());
                }
            })
        };

        let config = computed(move || settings.with(|settings| Rc::new(Config::of(settings))));
        let target = computed(move || settings.with(OrderBookSettings::center));
        // The feeds follow the books and the depth the settings draw, and
        // the ladder is let go when they lay it out anew.
        {
            let (feeds, ladder, mark) = (feeds.clone(), ladder.clone(), mark.clone());
            let laid_out = RefCell::new(None::<Rc<Config>>);
            effect(move || {
                let next = config.get();
                untracked(|| {
                    follow_layers(&ferris, &feeds, &next.layers, next.depth, &mark);
                    let relaid = laid_out
                        .borrow()
                        .as_ref()
                        .is_some_and(|before| before.relaid_by(&next));
                    if relaid {
                        ladder.borrow_mut().reset();
                    }
                    *laid_out.borrow_mut() = Some(next);
                });
                mark();
            });
        }
        {
            let (follow, mark) = (follow.clone(), mark.clone());
            effect(move || {
                follow.set(target.get());
                mark();
            });
        }

        {
            let (ladder, follow) = (ladder.clone(), follow.clone());
            scope.while_visible(move || {
                let refresh: Rc<dyn Fn()> = {
                    let (feeds, ladder) = (feeds.clone(), ladder.clone());
                    let (follow, dirty) = (follow.clone(), dirty.clone());
                    Rc::new(move || {
                        dirty.set(false);
                        let config = config.get_untracked();
                        let (books, failed): (Vec<_>, Vec<_>) = feeds.with(|feeds| {
                            feeds
                                .iter()
                                .map(|(_, feed)| (feed.book(), feed.failed()))
                                .unzip()
                        });
                        let (width, height) = size.get_untracked();
                        let mut next = ladder.borrow_mut().frame(
                            &config,
                            follow.get(),
                            &books,
                            f64::from(width),
                            f64::from(height),
                        );
                        next.failed = failed;
                        frame.try_set(Shared::new(next));
                    })
                };
                // New books and settings: at most once a frame.
                {
                    let (refresh, dirty) = (refresh.clone(), dirty.clone());
                    effect(move || {
                        wake.get();
                        if dirty.get() {
                            let (refresh, dirty) = (refresh.clone(), dirty.clone());
                            spawn(async move {
                                next_frame().await;
                                if dirty.get() {
                                    refresh();
                                }
                            });
                        }
                    });
                }
                // A resize or a move of the ladder: at once.
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
            ladder,
            follow,
            moved,
        }
    }

    /// Moves the ladder `rows` rows toward higher prices, as the user
    /// asked, which stops it following a book. Says whether it followed
    /// one until now: the view then saves that it follows none. A ladder
    /// running across the pane does not move (the old widget's rule).
    pub(super) fn pan(&self, rows: f64) -> bool {
        let frame = self.frame.get_untracked();
        if frame.0.config.layout != BookLayout::Vertical || !frame.0.steps() {
            return false;
        }
        self.ladder.borrow_mut().pan(&frame.0, rows);
        let followed = self.follow.replace(None).is_some();
        self.moved.update(|moves| *moves = moves.wrapping_add(1));
        followed
    }

    /// Centers the ladder again and fits its step to the prices, as a
    /// double click on the price column asks.
    pub(super) fn recenter(&self) {
        self.ladder.borrow_mut().reset();
        self.moved.update(|moves| *moves = moves.wrapping_add(1));
    }
}

/// Keeps one feed lease per layer, in their order: a book already
/// streamed at the levels `depth` asks for keeps its lease, a new one gets
/// a lease that marks the session dirty on each change, and the rest are
/// let go.
fn follow_layers(
    ferris: &Ferris,
    feeds: &Keyed<String, OrderBookFeed>,
    layers: &[Layer],
    depth: usize,
    mark: &Rc<dyn Fn()>,
) {
    let depth = u32::try_from(depth).unwrap_or(u32::MAX);
    feeds.follow(
        layers.iter().map(|layer| (layer.key.clone(), layer)),
        |_, _| true,
        |layer| {
            let mark = mark.clone();
            ferris.order_book(&layer.market, depth, move || mark())
        },
    );
}
