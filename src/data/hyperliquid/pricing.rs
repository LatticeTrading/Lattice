//! What an asset is priced at on Hyperliquid, followed live over the
//! WebSocket ([`super::stream`]): Hyperliquid as the app's prices ask for
//! it ([`PriceSource`]), either way a price is asked for ([`PriceKind`]):
//!
//! - its mark (`activeAssetCtx`'s `markPx`): the price Hyperliquid values
//!   a position at, margins it by and liquidates it on. Hyperliquid says it
//!   about once a second for each asset, which is its own pace: nothing
//!   here holds it back;
//! - its live price: the middle of its best bid and its best offer (`bbo`),
//!   said each time either changes, several times a second for a busy
//!   asset. While its book says neither (before the first is heard, a
//!   delisted asset's) it is the mark.
//!
//! One feed per asset ([`Priced`]), whatever its holds ask for: the first
//! hold ([`Hyperliquid::follow_asset_price`]) subscribes its context; later
//! holds share it and are told the price last heard at once; the last one
//! dropping lets it go (the sharing is the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s). Its best bid and offer are
//! subscribed while a hold asks for the live price, and let go with the
//! last such hold.
//!
//! **They are subscribed only once the asset's context has spoken.**
//! Hyperliquid answers the context of an asset it does not know with an
//! error, which fails that one topic; it answers the best bid and offer of
//! one by closing the connection (checked live 2026-11-22: `NOPE`, and
//! `KPEPE` for `kPEPE`). That connection carries every account followed
//! live, and the socket subscribes everything again when it reconnects, so
//! one such topic would end them all for as long as it is wanted. A context
//! that speaks is Hyperliquid saying the asset is one it knows.
//!
//! While a topic is not live (the socket is reconnecting, or Hyperliquid
//! refused the subscription) what it last said is forgotten, and the holds
//! are told the price they are left with: the last one heard is never said
//! to be the price now.
//!
//! Nothing is asked over HTTP for any of it. An account's report says what
//! its positions were priced at when it was made, which is what shows until
//! a price is heard and whenever none is.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use serde_json::Value;

use super::{
    Hyperliquid,
    accounts::VENUE,
    stream::{AssetFeed, Protocol, Topic},
};
use crate::data::{
    net::{FeedLease, FeedNode, Figure, Subscription, TopicPhase, WeakSocket},
    prices::{HearsPrice, PriceFeed, PriceKind, PriceSource},
};

/// Which of an asset's prices was heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heard {
    Mark,
    Mid,
}

/// One asset's prices: its topics, what each last said and who hears it.
#[derive(Default)]
pub(super) struct Priced {
    /// Its context, held while any hold of the asset is.
    context: RefCell<Option<Subscription>>,
    /// Its best bid and offer, held while a hold asks for the live price,
    /// once its context has spoken.
    book: RefCell<Option<Subscription>>,
    /// Whether its context ever spoke: Hyperliquid knows the asset.
    known: Cell<bool>,
    /// What it is marked at, as last heard; none while its context is not
    /// live.
    mark: Cell<Option<f64>>,
    /// The middle of its best bid and offer, as last heard; none while its
    /// book says neither or is not live.
    mid: Cell<Option<f64>>,
    hearers: RefCell<Vec<(u64, PriceKind, HearsPrice)>>,
    next_hearer: Cell<u64>,
}

impl Priced {
    /// The asset's price of `kind`, as it stands.
    fn price(&self, kind: PriceKind) -> Option<f64> {
        match kind {
            PriceKind::Mark => self.mark.get(),
            PriceKind::Live => self.mid.get().or(self.mark.get()),
        }
    }

    /// Whether a hold asks for the live price.
    fn wants_live(&self) -> bool {
        let hearers = self.hearers.borrow();
        hearers.iter().any(|(_, kind, _)| *kind == PriceKind::Live)
    }

    /// Adds a hold's hearer of `kind`, and gives its id.
    fn hear(&self, kind: PriceKind, hears: HearsPrice) -> u64 {
        let id = self.next_hearer.get();
        self.next_hearer.set(id + 1);
        self.hearers.borrow_mut().push((id, kind, hears));
        id
    }

    /// Keeps `price` as what `what` last said (`None`: nothing any more),
    /// and tells the hearers whose price that changes, in the order they
    /// took hold. A price said again is not told again.
    fn heard(&self, what: Heard, price: Option<f64>) {
        let known = match what {
            Heard::Mark => &self.mark,
            Heard::Mid => &self.mid,
        };
        if known.replace(price) == price {
            return;
        }
        let concerned: Vec<(PriceKind, HearsPrice)> = self
            .hearers
            .borrow()
            .iter()
            .filter(|(_, kind, _)| match (what, kind) {
                (Heard::Mid, PriceKind::Mark) => false,
                // A live price is the mark only while the book says none.
                (Heard::Mark, PriceKind::Live) => self.mid.get().is_none(),
                _ => true,
            })
            .map(|(_, kind, hears)| (*kind, hears.clone()))
            .collect();
        for (kind, hears) in concerned {
            hears(self.price(kind));
        }
    }
}

type Node = FeedNode<String, Priced>;

/// One hold on an asset's price. Dropping it stops its hearer; the last
/// hold that asked for the live price lets the best bid and offer go, and
/// the last hold of the asset ends its context.
pub(super) struct PriceHold {
    lease: FeedLease<String, Priced>,
    id: u64,
}

impl Drop for PriceHold {
    fn drop(&mut self) {
        let priced = self.lease.state();
        priced
            .hearers
            .borrow_mut()
            .retain(|(id, ..)| *id != self.id);
        if !priced.wants_live() {
            // Nobody reads the book any more.
            let book = priced.book.borrow_mut().take();
            drop(book);
            priced.mid.set(None);
        }
    }
}

/// A price a frame names as decimal text; `None` for what is no price
/// above zero.
fn price_of(named: Option<&Value>) -> Option<f64> {
    let Figure(price) = Figure::of(named?)?;
    (price.is_finite() && price > 0.0).then_some(price)
}

/// The price a context's frame says its asset is marked at.
fn mark_of(data: &Value) -> Option<f64> {
    price_of(data.get("ctx")?.get("markPx"))
}

/// The middle of the best bid and the best offer a frame names (`bbo`:
/// the bid, then the offer, either `null` while its side is empty), or the
/// one it names alone; `None` when it names neither.
fn mid_of(data: &Value) -> Option<f64> {
    let sides = data.get("bbo")?.as_array()?;
    let side = |at: usize| price_of(sides.get(at)?.get("px"));
    match (side(0), side(1)) {
        (Some(bid), Some(offer)) => Some((bid + offer) / 2.0),
        (one, other) => one.or(other),
    }
}

/// Whether a topic in `phase` says nothing now: what it last said is not
/// the price any more.
fn silent(phase: TopicPhase) -> bool {
    matches!(phase, TopicPhase::Connecting | TopicPhase::Failed)
}

/// Subscribes `coin`'s best bid and offer for the feed `node`, which holds
/// them, unless it does already. Only ever for an asset whose context has
/// spoken ([`Priced::known`]): see the module's note on why.
fn start_book(socket: &WeakSocket<Protocol>, node: &Rc<Node>, coin: &str) {
    let priced = node.state();
    if !priced.known.get() || priced.book.borrow().is_some() {
        return;
    }
    let Some(socket) = socket.upgrade() else {
        return;
    };
    let (heard, phased): (Weak<Node>, Weak<Node>) = (Rc::downgrade(node), Rc::downgrade(node));
    // Asked for on behalf of the feed's holds, as its context was.
    let subscription = node.scope(|| {
        socket.subscribe_with_phase(
            Topic::asset(AssetFeed::Book, coin),
            move |data: &Value| {
                if let Some(node) = heard.upgrade() {
                    node.state().heard(Heard::Mid, mid_of(data));
                }
            },
            move |phase| {
                if silent(phase)
                    && let Some(node) = phased.upgrade()
                {
                    node.state().heard(Heard::Mid, None);
                }
            },
        )
    });
    *priced.book.borrow_mut() = Some(subscription);
}

impl PriceSource for Hyperliquid {
    fn venue(&self) -> &'static str {
        VENUE
    }

    fn follow_price(
        &self,
        instrument: &str,
        kind: PriceKind,
        hears: HearsPrice,
    ) -> Option<PriceFeed> {
        let hold = self.follow_asset_price(instrument, kind, hears);
        Some(PriceFeed::new(hold))
    }
}

impl Hyperliquid {
    /// Follows the price of the asset `coin` that `kind` names while the
    /// hold is kept, and tells `hears` each one, or that there is none any
    /// more.
    fn follow_asset_price(&self, coin: &str, kind: PriceKind, hears: HearsPrice) -> PriceHold {
        let hub = self.feeds.hub::<String, Priced>();
        let lease = hub.lease(
            coin.to_owned(),
            Priced::default,
            |node| self.start_context(node, coin),
            || {},
        );
        let priced = lease.state();
        let id = priced.hear(kind, hears.clone());
        if kind == PriceKind::Live {
            // At once for an asset another hold's context has spoken of;
            // else when its own does.
            start_book(&self.socket.downgrade(), lease.node(), coin);
        }
        // What the holds before it have heard, it hears now.
        if let Some(price) = priced.price(kind) {
            hears(Some(price));
        }
        PriceHold { lease, id }
    }

    /// Subscribes `coin`'s context for the feed `node`, which holds it. Its
    /// callbacks hold the feed and the socket weakly: the feed holds the
    /// subscription, and the socket its callbacks.
    fn start_context(&self, node: &Rc<Node>, coin: &str) {
        let (heard, phased): (Weak<Node>, Weak<Node>) = (Rc::downgrade(node), Rc::downgrade(node));
        let (socket, asset) = (self.socket.downgrade(), coin.to_owned());
        let subscription = self.socket.subscribe_with_phase(
            Topic::asset(AssetFeed::Context, coin),
            move |data: &Value| {
                let Some(node) = heard.upgrade() else {
                    return;
                };
                let priced = node.state();
                // Hyperliquid knows the asset: its book may be asked for.
                priced.known.set(true);
                if let Some(mark) = mark_of(data) {
                    priced.heard(Heard::Mark, Some(mark));
                }
                if priced.wants_live() {
                    start_book(&socket, &node, &asset);
                }
            },
            move |phase| {
                if silent(phase)
                    && let Some(node) = phased.upgrade()
                {
                    node.state().heard(Heard::Mark, None);
                }
            },
        );
        *node.state().context.borrow_mut() = Some(subscription);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_contexts_frame_says_what_its_asset_is_marked_at() {
        // As the live socket sent it on 2026-11-22: decimal text.
        let frame = json!({ "coin": "BTC", "ctx": {
            "funding": "-0.0000094935", "oraclePx": "59343.0", "markPx": "59312.0",
            "midPx": "59311.0", "impactPxs": ["59310.0", "59323.9"],
        } });
        assert_eq!(mark_of(&frame), Some(59_312.0));
        // A delisted asset has no mid, and still a mark.
        let delisted = json!({ "coin": "MATIC", "ctx": { "markPx": "0.37621", "midPx": null } });
        assert_eq!(mark_of(&delisted), Some(0.376_21));
        // What names no price above zero says nothing.
        for nothing in [
            json!({ "coin": "BTC" }),
            json!({ "coin": "BTC", "ctx": {} }),
            json!({ "coin": "BTC", "ctx": { "markPx": null } }),
            json!({ "coin": "BTC", "ctx": { "markPx": "0.0" } }),
            json!({ "coin": "BTC", "ctx": { "markPx": "soon" } }),
        ] {
            assert_eq!(mark_of(&nothing), None, "{nothing}");
        }
    }

    #[test]
    fn a_best_bid_and_offer_say_the_middle_of_them() {
        // As the live socket sent it on 2026-11-22: the bid, then the offer.
        let frame = json!({ "coin": "BTC", "time": 1_795_323_615_652_u64, "bbo": [
            { "px": "59289.0", "sz": "2.02666", "n": 16 },
            { "px": "59290.0", "sz": "7.38389", "n": 20 },
        ] });
        assert_eq!(mid_of(&frame), Some(59_289.5));
        // One side empty: the side there is.
        let bid = json!({ "coin": "BTC", "bbo": [{ "px": "59289.0", "sz": "1.0", "n": 1 }, null] });
        assert_eq!(mid_of(&bid), Some(59_289.0));
        let offer =
            json!({ "coin": "BTC", "bbo": [null, { "px": "59290.0", "sz": "1.0", "n": 1 }] });
        assert_eq!(mid_of(&offer), Some(59_290.0));
        // Neither, and what is no such frame: no price.
        for nothing in [
            json!({ "coin": "BTC", "bbo": [null, null] }),
            json!({ "coin": "BTC", "bbo": [] }),
            json!({ "coin": "BTC", "bbo": [{ "px": "0.0" }, { "px": "soon" }] }),
            json!({ "coin": "BTC" }),
        ] {
            assert_eq!(mid_of(&nothing), None, "{nothing}");
        }
    }

    /// A hearer of `kind` of `priced`, and what it was told so far.
    fn hearer(priced: &Priced, kind: PriceKind) -> Rc<RefCell<Vec<Option<f64>>>> {
        let told: Rc<RefCell<Vec<Option<f64>>>> = Rc::default();
        let tells = told.clone();
        priced.hear(kind, Rc::new(move |price| tells.borrow_mut().push(price)));
        told
    }

    #[test]
    fn the_live_price_is_the_books_middle_and_the_mark_while_there_is_none() {
        let priced = Priced::default();
        let (live, mark) = (
            hearer(&priced, PriceKind::Live),
            hearer(&priced, PriceKind::Mark),
        );
        assert!(priced.wants_live());

        // Until the book speaks, the mark is both prices.
        priced.heard(Heard::Mark, Some(100.0));
        assert_eq!(*live.borrow(), [Some(100.0)]);
        assert_eq!(*mark.borrow(), [Some(100.0)]);
        // The book speaks: the live price is its middle from then on, and a
        // mark heard meanwhile moves the mark alone.
        priced.heard(Heard::Mid, Some(100.5));
        priced.heard(Heard::Mark, Some(101.0));
        priced.heard(Heard::Mid, Some(102.5));
        assert_eq!(*live.borrow(), [Some(100.0), Some(100.5), Some(102.5)]);
        assert_eq!(*mark.borrow(), [Some(100.0), Some(101.0)]);
        assert_eq!(priced.price(PriceKind::Live), Some(102.5));
        assert_eq!(priced.price(PriceKind::Mark), Some(101.0));

        // A price said again is not told again.
        priced.heard(Heard::Mid, Some(102.5));
        priced.heard(Heard::Mark, Some(101.0));
        assert_eq!((live.borrow().len(), mark.borrow().len()), (3, 2));

        // The book says neither side any more: the mark again.
        priced.heard(Heard::Mid, None);
        assert_eq!(live.borrow().last(), Some(&Some(101.0)));
        assert_eq!(mark.borrow().len(), 2);
        // And with no mark either, no price: never the last one heard.
        priced.heard(Heard::Mark, None);
        assert_eq!(live.borrow().last(), Some(&None));
        assert_eq!(mark.borrow().last(), Some(&None));
        assert_eq!(priced.price(PriceKind::Live), None);

        // A hold that asks for the mark alone wants no book.
        let marked = Priced::default();
        let _mark = hearer(&marked, PriceKind::Mark);
        assert!(!marked.wants_live());
    }
}
