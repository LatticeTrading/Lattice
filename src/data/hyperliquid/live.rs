//! An account followed live: what Hyperliquid's WebSocket says of it as it
//! happens ([`super::stream`]), given to the requests that read the same
//! things, so that every widget watching the account shows it at once.
//!
//! The first hold of an account ([`Hyperliquid::follow_account`]) subscribes
//! its three topics; later holds share them, and the last one dropping lets
//! them go (the sharing is the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s). While they are held:
//!
//! - its open orders, sent whole each time one is placed, filled or
//!   cancelled, become the value of the request for them
//!   ([`Info::FrontendOpenOrders`]);
//! - its perpetuals account, sent whole as it changes, becomes the value of
//!   the request for it ([`Info::ClearinghouseState`]);
//! - its fills are added to the value of the request for its trades
//!   ([`Info::UserFills`]), when something has asked for them: the snapshot
//!   a subscription opens with fills in what was missed while there was no
//!   connection, and each fill after it is also told to whoever holds the
//!   account ([`AccountEvent::Filled`]), oldest first.
//!
//! A value given this way counts as a fetch
//! ([`Queries::supply`](crate::data::net::Queries::supply)): the request's
//! poll waits an interval from it, so an account whose stream is busy is
//! not polled, and one whose stream has stopped is caught up within a poll.
//! Nothing here keeps a second copy of the account.
//!
//! A frame that does not read as what its channel sends is logged and left
//! out; the polls still say what is so.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
};

use serde::Deserialize;
use serde_json::Value;

use super::{
    Hyperliquid, Info,
    fills::UserFills,
    orders::FrontendOpenOrders,
    perps::ClearinghouseState,
    stream::{Channel, Topic},
};
use crate::data::{
    net::{FeedLease, FeedNode, Queries, Subscription, WeakQueries},
    portfolio::{Fill, Fills, MarginAccount},
    trading::{AccountEvent, Hears, OpenOrders},
};

/// The most trades the request for an account's keeps: what Hyperliquid
/// answers it with at most.
const MAX_FILLS: usize = 2000;

/// One account's feed: its topics, and who hears what happens to it.
#[derive(Default)]
pub(super) struct Live {
    /// Held while any hold of the account is.
    subscriptions: RefCell<Vec<Subscription>>,
    hearers: RefCell<Vec<(u64, Hears)>>,
    next_hearer: Cell<u64>,
}

impl Live {
    /// Tells every hold's hearer `event`, in the order they took hold.
    fn tell(&self, event: &AccountEvent) {
        let hearers: Vec<Hears> = self
            .hearers
            .borrow()
            .iter()
            .map(|(_, hears)| hears.clone())
            .collect();
        for hears in hearers {
            hears(event);
        }
    }
}

type Node = FeedNode<String, Live>;

/// One hold on a followed account. Dropping it stops its hearer, and the
/// last hold of the account ends its topics.
pub(super) struct Hold {
    lease: FeedLease<String, Live>,
    id: u64,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.lease
            .state()
            .hearers
            .borrow_mut()
            .retain(|(id, _)| *id != self.id);
    }
}

/// `data` read as `T`, or nothing and a line in the log.
fn read<'de, T: Deserialize<'de>>(data: &'de Value, what: &str) -> Option<T> {
    T::deserialize(data)
        .map_err(|error| log::warn!("Hyperliquid realtime: unreadable {what}: {error}"))
        .ok()
}

/// `known` with those of `arrived` it does not hold yet, newest first;
/// `None` when it holds them all.
fn with_new(known: &Fills, arrived: &[Fill]) -> Option<Fills> {
    let held: HashSet<&str> = known.0.iter().map(|fill| fill.id.as_str()).collect();
    let mut fills: Vec<Fill> = arrived
        .iter()
        .filter(|fill| !held.contains(fill.id.as_str()))
        .cloned()
        .collect();
    if fills.is_empty() {
        return None;
    }
    fills.extend(known.0.iter().cloned());
    fills.sort_by(|a, b| b.time_ms.total_cmp(&a.time_ms));
    fills.truncate(MAX_FILLS);
    Some(Fills(fills))
}

/// The open orders of `user`, as their topic sent them.
fn orders(queries: &Queries, user: &str, data: &Value) {
    let Some(orders) = data
        .get("orders")
        .and_then(|orders| read::<FrontendOpenOrders>(orders, "open orders"))
    else {
        return;
    };
    let request = Info::FrontendOpenOrders {
        user: user.to_owned(),
    };
    queries.supply(&request.key(), OpenOrders::from(orders));
}

/// The perpetuals account of `user`, as its topic sent it.
fn account(queries: &Queries, user: &str, data: &Value) {
    let Some(state) = data
        .get("clearinghouseState")
        .and_then(|state| read::<ClearinghouseState>(state, "perpetuals account"))
    else {
        return;
    };
    let request = Info::ClearinghouseState {
        user: user.to_owned(),
        dex: None,
    };
    queries.supply(&request.key(), MarginAccount::from(state));
}

/// Fills of `user`, as their topic sent them: added to the trades asked
/// for, and told to the account's holds unless they are the snapshot.
fn fills(queries: &Queries, live: &Live, user: &str, data: &Value) {
    let Some(arrived) = data
        .get("fills")
        .and_then(|fills| read::<UserFills>(fills, "fills"))
    else {
        return;
    };
    let Fills(arrived) = arrived.into();
    let request = Info::UserFills {
        user: user.to_owned(),
        aggregate_by_time: true,
    };
    let key = request.key();
    // Only into trades something asked for: nobody else has a list to add
    // them to, and whoever asks next is answered with them.
    if let Some(known) = queries.cached::<Fills>(&key)
        && let Some(fills) = with_new(&known, &arrived)
    {
        queries.supply(&key, fills);
    }
    let snapshot = data.get("isSnapshot").and_then(Value::as_bool) == Some(true);
    if !snapshot {
        // Newest first as read; told in the order they happened.
        for fill in arrived.into_iter().rev() {
            live.tell(&AccountEvent::Filled(fill));
        }
    }
}

impl Hyperliquid {
    /// Follows the account at `address` live while the hold is kept, and
    /// tells `hears` what happens to it.
    pub(super) fn follow_account(&self, address: &str, hears: Hears) -> Hold {
        let user = address.to_ascii_lowercase();
        let hub = self.feeds.hub::<String, Live>();
        let lease = hub.lease(
            user.clone(),
            Live::default,
            |node| self.start(node, &user),
            || {},
        );
        let live = lease.state();
        let id = live.next_hearer.get();
        live.next_hearer.set(id + 1);
        live.hearers.borrow_mut().push((id, hears));
        Hold { lease, id }
    }

    /// Subscribes `user`'s topics for the feed `node`, which holds them.
    /// Their callbacks hold the hub and the feed weakly: the feed holds
    /// the subscriptions, and the socket their callbacks.
    fn start(&self, node: &Rc<Node>, user: &str) {
        let subscribe = |channel: Channel, takes: fn(&Queries, &Live, &str, &Value)| {
            let (queries, node, user): (WeakQueries, Weak<Node>, String) = (
                self.queries.downgrade(),
                Rc::downgrade(node),
                user.to_owned(),
            );
            self.socket
                .subscribe(Topic::new(channel, &user), move |data: &Value| {
                    if let (Some(queries), Some(node)) = (queries.upgrade(), node.upgrade()) {
                        takes(&queries, node.state(), &user, data);
                    }
                })
        };
        let subscriptions = vec![
            subscribe(Channel::OpenOrders, |queries, _, user, data| {
                orders(queries, user, data);
            }),
            subscribe(Channel::ClearinghouseState, |queries, _, user, data| {
                account(queries, user, data);
            }),
            subscribe(Channel::UserFills, fills),
        ];
        *node.state().subscriptions.borrow_mut() = subscriptions;
    }
}

#[cfg(test)]
mod tests {
    use crate::data::portfolio::TradeSide;

    use super::*;

    fn fill(id: &str, time_ms: f64) -> Fill {
        Fill {
            id: id.to_owned(),
            time_ms,
            instrument: "BTC".to_owned(),
            side: TradeSide::Buy,
            action: None,
            price: 61_000.0,
            size: 0.01,
            fee: None,
            fee_asset: None,
            closed_pnl: None,
            taker: None,
        }
    }

    #[test]
    fn new_fills_join_the_trades_newest_first_and_each_once() {
        let ids =
            |fills: &Fills| -> Vec<String> { fills.0.iter().map(|fill| fill.id.clone()).collect() };
        let known = Fills(vec![fill("3", 30.0), fill("2", 20.0), fill("1", 10.0)]);
        // One it holds and two it does not, one of them older than its
        // newest (what a snapshot after a gap brings).
        let arrived = [fill("5", 50.0), fill("3", 30.0), fill("4", 25.0)];
        let fills = with_new(&known, &arrived).unwrap();
        assert_eq!(ids(&fills), ["5", "3", "4", "2", "1"]);
        // Nothing new is nothing to give.
        assert_eq!(with_new(&fills, &arrived), None);
        assert_eq!(with_new(&known, &[]), None);

        // No more are kept than the request answers with.
        let many = Fills(
            (0..MAX_FILLS)
                .map(|at| fill(&format!("old-{at}"), at as f64))
                .collect(),
        );
        let fills = with_new(&many, &[fill("new", 1e9)]).unwrap();
        assert_eq!(fills.0.len(), MAX_FILLS);
        assert_eq!(fills.0[0].id, "new");
    }
}
