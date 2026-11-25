//! What a market is priced at on Lighter, followed live over the WebSocket
//! ([`super::stream`]): Lighter as the app's prices ask for it
//! ([`PriceSource`]), either way a price is asked for ([`PriceKind`]):
//!
//! - its mark (`mark_price`): the price Lighter values a position at,
//!   margins it by and liquidates it on;
//! - its live price: the middle of its best bid and its best offer
//!   (`mid_price`, or the middle of `best_bid_price` and `best_ask_price`
//!   where it is left out), and its mark while its book says neither.
//!
//! Both come in one frame of the market's statistics, which Lighter sends
//! about once a second: that is its own pace for either, and nothing here
//! holds it back.
//!
//! One feed per market ([`Priced`]), whatever its holds ask for: the first
//! hold ([`Lighter::follow_market_price`]) subscribes its statistics; later
//! holds share them and are told the price last heard at once; the last one
//! dropping lets them go (the sharing is the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s).
//!
//! A market is subscribed by its index, and named here by its symbol. The
//! index is known for every market an account read here has an entry for,
//! and for all of them once the markets were read
//! ([`Directory`](super::markets::Directory)); for one that is in neither
//! yet, the markets are asked for first, and a symbol Lighter lists no
//! market for is followed with nothing ever said.
//!
//! While the topic is not live (the socket is reconnecting, or Lighter
//! refused the subscription) what it last said is forgotten, and the holds
//! are told there is no price: the last one heard is never said to be the
//! price now.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use serde_json::Value;

use super::{
    Lighter,
    accounts::VENUE,
    stream::{Protocol, Topic},
};
use crate::data::{
    net::{FeedLease, FeedNode, Figure, Subscription, TaskHandle, TopicPhase, WeakSocket},
    prices::{HearsPrice, PriceFeed, PriceKind, PriceSource},
};

/// One market's prices: its topic, what it last said and who hears it.
#[derive(Default)]
pub(super) struct Priced {
    /// What asks for the markets first, for one whose index is not known.
    starting: RefCell<Option<TaskHandle>>,
    /// Its statistics, held while any hold of the market is.
    statistics: RefCell<Option<Subscription>>,
    /// What it is marked at, as last heard; none while its topic is not
    /// live.
    mark: Cell<Option<f64>>,
    /// The middle of its best bid and offer, as last heard; none while its
    /// book says neither or its topic is not live.
    mid: Cell<Option<f64>>,
    hearers: RefCell<Vec<(u64, PriceKind, HearsPrice)>>,
    next_hearer: Cell<u64>,
}

impl Priced {
    /// The market's price of `kind`, as it stands.
    fn price(&self, kind: PriceKind) -> Option<f64> {
        match kind {
            PriceKind::Mark => self.mark.get(),
            PriceKind::Live => self.mid.get().or(self.mark.get()),
        }
    }

    /// Keeps `mark` and `mid` as what the market last said (`None`: nothing
    /// any more), and tells each hearer whose price that changes, in the
    /// order they took hold. A price said again is not told again.
    fn heard(&self, mark: Option<f64>, mid: Option<f64>) {
        let before = (self.price(PriceKind::Mark), self.price(PriceKind::Live));
        self.mark.set(mark);
        self.mid.set(mid);
        let was = |kind: PriceKind| match kind {
            PriceKind::Mark => before.0,
            PriceKind::Live => before.1,
        };
        let concerned: Vec<(PriceKind, HearsPrice)> = self
            .hearers
            .borrow()
            .iter()
            .filter(|(_, kind, _)| was(*kind) != self.price(*kind))
            .map(|(_, kind, hears)| (*kind, hears.clone()))
            .collect();
        for (kind, hears) in concerned {
            hears(self.price(kind));
        }
    }
}

type Node = FeedNode<String, Priced>;

/// One hold on a market's price. Dropping it stops its hearer, and the last
/// hold of the market ends its topic.
pub(super) struct PriceHold {
    lease: FeedLease<String, Priced>,
    id: u64,
}

impl Drop for PriceHold {
    fn drop(&mut self) {
        self.lease
            .state()
            .hearers
            .borrow_mut()
            .retain(|(id, ..)| *id != self.id);
    }
}

/// A price a frame names as decimal text; `None` for what is no price
/// above zero.
fn price_of(named: Option<&Value>) -> Option<f64> {
    let Figure(price) = Figure::of(named?)?;
    (price.is_finite() && price > 0.0).then_some(price)
}

/// What a frame of a market's statistics says it is marked at, and the
/// middle of its best bid and offer: as Lighter says it, else of the two
/// it names, else the one it names alone.
fn prices_of(frame: &Value) -> (Option<f64>, Option<f64>) {
    let Some(statistics) = frame.get("market_stats") else {
        return (None, None);
    };
    let named = |member: &str| price_of(statistics.get(member));
    let mid =
        named("mid_price").or_else(
            || match (named("best_bid_price"), named("best_ask_price")) {
                (Some(bid), Some(offer)) => Some((bid + offer) / 2.0),
                (one, other) => one.or(other),
            },
        );
    (named("mark_price"), mid)
}

/// Subscribes the statistics of the market `id`, named `symbol`, for the
/// feed `node`, which holds them. Its callbacks hold the feed weakly: the
/// feed holds the subscription, and the socket its callbacks.
fn subscribe(socket: &WeakSocket<Protocol>, node: &Rc<Node>, id: u64, symbol: &str) {
    let Some(socket) = socket.upgrade() else {
        return;
    };
    let (heard, phased): (Weak<Node>, Weak<Node>) = (Rc::downgrade(node), Rc::downgrade(node));
    // Asked for on behalf of the feed's holds, whenever it comes to be.
    let subscription = node.scope(|| {
        socket.subscribe_with_phase(
            Topic::market(id, symbol),
            move |frame: &Value| {
                if let Some(node) = heard.upgrade() {
                    let (mark, mid) = prices_of(frame);
                    node.state().heard(mark, mid);
                }
            },
            move |phase| {
                // It says nothing now: what it last said is not the price
                // any more.
                if matches!(phase, TopicPhase::Connecting | TopicPhase::Failed)
                    && let Some(node) = phased.upgrade()
                {
                    node.state().heard(None, None);
                }
            },
        )
    });
    *node.state().statistics.borrow_mut() = Some(subscription);
}

impl PriceSource for Lighter {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn follow_price(
        &self,
        instrument: &str,
        kind: PriceKind,
        hears: HearsPrice,
    ) -> Option<PriceFeed> {
        let hold = self.follow_market_price(instrument, kind, hears);
        Some(PriceFeed::new(hold))
    }
}

impl Lighter {
    /// Follows the price of the market named `symbol` that `kind` names
    /// while the hold is kept, and tells `hears` each one, or that there is
    /// none any more.
    fn follow_market_price(&self, symbol: &str, kind: PriceKind, hears: HearsPrice) -> PriceHold {
        let hub = self.feeds.hub::<String, Priced>();
        let lease = hub.lease(
            symbol.to_owned(),
            Priced::default,
            |node| self.start_prices(node, symbol),
            || {},
        );
        let priced = lease.state();
        let id = priced.next_hearer.get();
        priced.next_hearer.set(id + 1);
        priced.hearers.borrow_mut().push((id, kind, hears.clone()));
        // What the holds before it have heard, it hears now.
        if let Some(price) = priced.price(kind) {
            hears(Some(price));
        }
        PriceHold { lease, id }
    }

    /// Subscribes `symbol`'s statistics for the feed `node`: at once when
    /// its market's index is known, else once the markets have said it.
    fn start_prices(&self, node: &Rc<Node>, symbol: &str) {
        let socket = self.socket.downgrade();
        if let Some(id) = self.api.directory.id(symbol) {
            subscribe(&socket, node, id, symbol);
            return;
        }
        let (this, node_weak, symbol) = (self.clone(), Rc::downgrade(node), symbol.to_owned());
        let task = TaskHandle::spawn(async move {
            let listed = this.markets().await;
            // The client is not held past the asking: the feed holds this
            // task, and the client the feed's hub.
            drop(this);
            let (Ok(markets), Some(node)) = (listed, node_weak.upgrade()) else {
                return;
            };
            if let Some(market) = markets.find(&symbol) {
                subscribe(&socket, &node, market.id, &symbol);
            }
        });
        *node.state().starting.borrow_mut() = Some(task);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_markets_statistics_say_its_mark_and_the_middle_of_its_book() {
        // As the live socket sent BTC's on 2026-11-22, cut to what is read.
        let frame = json!({
            "channel": "market_stats:1",
            "market_stats": {
                "symbol": "BTC", "market_id": 1, "index_price": "61269.0",
                "mark_price": "61260.8", "mid_price": "61245.2",
                "best_ask_price": "61245.6", "best_bid_price": "61244.7",
                "last_trade_price": "61248.6", "current_funding_rate": "0.0012",
            },
            "type": "update/market_stats",
        });
        assert_eq!(prices_of(&frame), (Some(61_260.8), Some(61_245.2)));
        // Without the middle said: of the two sides, or the one there is.
        let sides = |bid: Value, offer: Value| {
            json!({ "market_stats": { "mark_price": "100.0", "best_bid_price": bid,
                                      "best_ask_price": offer } })
        };
        assert_eq!(
            prices_of(&sides(json!("99.0"), json!("101.0"))),
            (Some(100.0), Some(100.0))
        );
        assert_eq!(
            prices_of(&sides(json!("99.0"), json!(""))),
            (Some(100.0), Some(99.0))
        );
        assert_eq!(
            prices_of(&sides(json!(null), json!("0"))),
            (Some(100.0), None)
        );
        // What is no price is none, and neither is a frame of something
        // else.
        let none = json!({ "market_stats": { "mark_price": "0", "mid_price": "" } });
        assert_eq!(prices_of(&none), (None, None));
        assert_eq!(prices_of(&json!({ "type": "connected" })), (None, None));
    }

    /// What was heard, by whoever asked for which price.
    type Heard = Vec<(PriceKind, Option<f64>)>;

    #[test]
    fn each_hold_hears_the_price_it_asked_for_when_it_changes() {
        let priced = Priced::default();
        let heard: Rc<RefCell<Heard>> = Rc::default();
        for (id, kind) in [(0, PriceKind::Mark), (1, PriceKind::Live)] {
            let heard = heard.clone();
            let hears: HearsPrice = Rc::new(move |price| heard.borrow_mut().push((kind, price)));
            priced.hearers.borrow_mut().push((id, kind, hears));
        }
        let take = || std::mem::take(&mut *heard.borrow_mut());

        // The mark alone: the live price is the mark while the book says
        // nothing.
        priced.heard(Some(100.0), None);
        assert_eq!(
            take(),
            [
                (PriceKind::Mark, Some(100.0)),
                (PriceKind::Live, Some(100.0))
            ]
        );
        // The book speaks: the live price is its middle, and the mark is
        // not said again.
        priced.heard(Some(100.0), Some(100.5));
        assert_eq!(take(), [(PriceKind::Live, Some(100.5))]);
        // The same again is no news.
        priced.heard(Some(100.0), Some(100.5));
        assert_eq!(take(), []);
        priced.heard(Some(101.0), Some(100.5));
        assert_eq!(take(), [(PriceKind::Mark, Some(101.0))]);
        // The topic stopped: neither is the price any more.
        priced.heard(None, None);
        assert_eq!(take(), [(PriceKind::Mark, None), (PriceKind::Live, None)]);
        assert_eq!(priced.price(PriceKind::Live), None);
    }
}
