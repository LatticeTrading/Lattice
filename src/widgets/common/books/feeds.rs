//! The feeds of the books a widget draws ([`BookFeeds`]): a lease per
//! layer, and when what they brought is drawn.
//!
//! A feed tells its lease of each change from the socket's callback; the
//! lease only marks the widget dirty there. While the widget shows, it
//! draws at most once a frame ([`BookFeeds::each_frame`]).

use std::{cell::Cell, rc::Rc};

use nsh_core::prelude::*;

use super::Layer;
use crate::data::{
    Keyed,
    ferris::{BookTraits, Ferris, MarketSelection, OrderBook, OrderBookFeed},
};

/// One feed lease per book a widget draws, and whether they brought
/// something it has not drawn.
pub(crate) struct BookFeeds {
    ferris: Ferris,
    feeds: Keyed<String, OrderBookFeed>,
    /// Books arrived (or the widget asked) since they were last taken.
    dirty: Rc<Cell<bool>>,
    /// Bumped when `dirty` is set, so a visible widget schedules a frame.
    wake: Signal<u64>,
    mark: Rc<dyn Fn()>,
    /// What each book's newest state says of its amounts and liquidity, by
    /// its layer's key, as last taken.
    traits: Signal<Vec<(String, BookTraits)>>,
}

impl BookFeeds {
    /// No feed yet. Made under the session's owner, which its signal lives
    /// as long as.
    pub fn new(ferris: Ferris) -> Rc<Self> {
        let dirty = Rc::new(Cell::new(true));
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
        Rc::new(Self {
            ferris,
            feeds: Keyed::default(),
            dirty,
            wake,
            mark,
            traits: signal(Vec::new()),
        })
    }

    /// Keeps one lease per layer, in their order: a book already streamed
    /// at the levels `depth` asks for keeps its lease, a new one gets a
    /// lease that marks the widget dirty on each change, and the rest are
    /// let go.
    pub fn follow(&self, layers: &[Layer], depth: usize) {
        let books = layers
            .iter()
            .map(|layer| (layer.key.clone(), &*layer.market));
        self.follow_markets(books, depth);
    }

    /// [`Self::follow`] for any number of markets (the Price Delta
    /// Monitor's): one lease per market of `markets`, in their order, each
    /// given with what tells its stream apart at `depth`
    /// ([`stream_key`](crate::data::ferris::order_book::stream_key)).
    pub fn follow_markets<'a>(
        &self,
        markets: impl IntoIterator<Item = (String, &'a MarketSelection)>,
        depth: usize,
    ) {
        let depth = u32::try_from(depth).unwrap_or(u32::MAX);
        self.feeds.follow(
            markets,
            |_, _| true,
            |market| {
                let mark = self.mark.clone();
                self.ferris.order_book(market, depth, move || mark())
            },
        );
    }

    /// Says something changed that the widget has not drawn.
    pub fn mark(&self) {
        (self.mark)();
    }

    /// Each book's newest state and whether its topic was refused, in the
    /// order they are followed, taken to be drawn: nothing is new until the
    /// next change. [`Self::traits`] follows what they say.
    pub fn take(&self) -> (Vec<Option<Rc<OrderBook>>>, Vec<bool>) {
        self.dirty.set(false);
        self.feeds.with(|feeds| {
            let (books, failed): (Vec<_>, Vec<_>) = feeds
                .iter()
                .map(|(_, feed)| (feed.book(), feed.failed()))
                .unzip();
            let said = || {
                feeds
                    .iter()
                    .zip(&books)
                    .filter_map(|((key, _), book)| Some((key, &book.as_ref()?.traits)))
            };
            let same = self.traits.with_untracked(|kept| {
                kept.len() == said().count()
                    && kept
                        .iter()
                        .zip(said())
                        .all(|((key, traits), (now, says))| key == now && traits == says)
            });
            if !same {
                self.traits.try_set(
                    said()
                        .map(|(key, traits)| (key.clone(), traits.clone()))
                        .collect(),
                );
            }
            (books, failed)
        })
    }

    /// What the books taken last say of their amounts and liquidity, by
    /// their layers' keys (what [`notices`](super::notices) reads).
    pub fn traits(&self) -> Signal<Vec<(String, BookTraits)>> {
        self.traits
    }

    /// Runs `draw` a frame after something changed, at most once a frame,
    /// under the current owner: call it where the widget's rendering work
    /// starts (`SessionScope::while_visible`). `draw` takes the books
    /// ([`Self::take`]).
    pub fn each_frame(&self, draw: Rc<dyn Fn()>) {
        let (wake, dirty) = (self.wake, self.dirty.clone());
        effect(move || {
            wake.get();
            if dirty.get() {
                let (draw, dirty) = (draw.clone(), dirty.clone());
                spawn(async move {
                    next_frame().await;
                    if dirty.get() {
                        draw();
                    }
                });
            }
        });
    }
}
