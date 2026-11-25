//! An account followed live: what Lighter's WebSocket says of it as it
//! happens ([`super::stream`]), given to the requests that read the same
//! things, so that every widget watching the account shows it at once.
//!
//! What is followed is the address's main account, the one orders are
//! placed for ([`AccountStates::trading`](super::state)). The first hold of
//! an address ([`Lighter::follow_account`]) asks which account that is and
//! subscribes its topics; later holds share them, and the last one dropping
//! lets them go (the sharing is the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s). While they are held:
//!
//! - its trades are added to the value of the request for its trades, when
//!   something has asked for them: the latest ones a subscription opens
//!   with fill in what was missed while there was no connection, and each
//!   trade after them is also told to whoever holds the account
//!   ([`AccountEvent::Filled`]), oldest first;
//! - its positions are sent only where they changed, and an address's
//!   report is of all its accounts together: so a change has the accounts
//!   asked for again a moment later
//!   ([`refresh_soon`](super::accounts::refresh_soon)), and a trade does
//!   the same, rather than a second copy of the account being kept and
//!   patched here;
//! - its orders are sent as each changes, and only to a token of the
//!   account's own key: so they are followed once trading is enabled for
//!   the address ([`super::key`]), and applied to its orders as last read
//!   ([`super::orders`]), which become the value of the request for them.
//!   A feed with them is another feed than one without: whoever follows an
//!   account asks again when trading comes to be enabled for it, and the
//!   topics both have are shared underneath.
//!
//! A value given this way counts as a fetch
//! ([`Queries::supply`](crate::data::net::Queries::supply)): the request's
//! poll waits an interval from it.
//!
//! A subscription carries a token good for an hour. The orders' is asked
//! for afresh well before that ([`RENEW_EVERY`]), with a new token: nothing
//! says what Lighter does with a subscription whose token has run out.
//!
//! A frame that does not read as what its channel sends is logged and left
//! out; the polls still say what is so.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
    time::Duration,
};

use nsh_core::prelude::*;
use serde_json::Value;

use super::{
    Api, Kept, Lighter,
    accounts::{Cause, accounts_of, refresh_soon, report_key},
    fills, orders,
    stream::{Channel, Topic, is_snapshot},
    trading::orders_key,
};
use crate::data::{
    net::{FeedLease, FeedNode, Queries, Subscription, TaskHandle, WeakQueries},
    portfolio::{Fill, Fills},
    trading::{AccountEvent, Hears},
};

/// The most trades the request for an address's keeps once the stream has
/// added to them.
const MAX_FILLS: usize = 1000;
/// How long after Lighter could not say which account an address trades
/// with it is asked again, at first; twice as long each time, up to
/// [`LAST_RETRY`].
const FIRST_RETRY: Duration = Duration::from_secs(5);
const LAST_RETRY: Duration = Duration::from_secs(60);
/// How often the orders are subscribed afresh, with a new token.
const RENEW_EVERY: Duration = Duration::from_secs(45 * 60);

/// One address's feed: its topics, and who hears what happens to it.
#[derive(Default)]
pub(super) struct Live {
    /// What finds the account and subscribes its topics.
    starting: RefCell<Option<TaskHandle>>,
    /// Held while any hold of the address is.
    subscriptions: RefCell<Vec<Subscription>>,
    /// The orders, which are renewed: held as the others are.
    orders: RefCell<Option<Subscription>>,
    renewing: RefCell<Option<TaskHandle>>,
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

/// What tells two feeds apart: the address, and whether its orders are
/// followed with it.
type Followed = (String, bool);
type Node = FeedNode<Followed, Live>;

/// One hold on a followed address. Dropping it stops its hearer, and the
/// last hold of the address ends its topics.
pub(super) struct Hold {
    lease: FeedLease<Followed, Live>,
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

/// Whether a frame of an account's positions names any: one that changed,
/// or all of them.
fn names_positions(frame: &Value) -> bool {
    let positions = frame.get("positions").and_then(Value::as_object);
    positions.is_some_and(|positions| !positions.is_empty())
}

/// What a feed's callbacks reach the client by: everything held weakly, or
/// holding nothing that holds the socket.
#[derive(Clone)]
struct Reach {
    api: Api,
    queries: WeakQueries,
    kept: Weak<Kept>,
    node: Weak<Node>,
    /// The address followed.
    user: String,
}

impl Reach {
    /// Has the address's accounts asked for again a moment from now.
    fn refresh(&self) {
        if let Some(kept) = self.kept.upgrade() {
            let stream = Cause::Stream;
            refresh_soon(&self.api, &self.queries, &kept, &self.user, &[], stream);
        }
    }

    /// The trades of the account `account`, as their topic sent them: added
    /// to the trades asked for, and told to the address's holds unless they
    /// are what the subscription opened with.
    fn trades(&self, queries: &Queries, account: u64, frame: &Value) {
        let arrived = fills::streamed(frame, account);
        if arrived.is_empty() {
            return;
        }
        let key = report_key("fills", &self.user);
        // Only into trades something asked for: nobody else has a list to
        // add them to, and whoever asks next is answered with them.
        if let Some(known) = queries.cached::<Fills>(&key)
            && let Some(fills) = with_new(&known, &arrived)
        {
            queries.supply(&key, fills);
        }
        if is_snapshot(frame) {
            return;
        }
        if let Some(node) = self.node.upgrade() {
            // Newest first as read; told in the order they happened.
            for fill in arrived.into_iter().rev() {
                node.state().tell(&AccountEvent::Filled(fill));
            }
        }
        // What traded changed what the account holds.
        self.refresh();
    }

    /// The orders that changed, as their topic sent them: applied to the
    /// address's orders as last read, which become the value of the request
    /// for them. Nothing is made of them before a request has said what the
    /// orders are: a change is applied to something.
    fn orders(&self, queries: &Queries, frame: &Value) {
        let (Some(changed), Some(kept)) = (orders::changed(frame), self.kept.upgrade()) else {
            return;
        };
        let listed = {
            let mut all = kept.orders.borrow_mut();
            let Some(known) = all.get_mut(&self.user) else {
                return;
            };
            *known = known.apply(changed);
            known.listed(&self.api.directory)
        };
        queries.supply(&orders_key(&self.user), listed);
    }
}

impl Lighter {
    /// Follows the address `address` live while the hold is kept, and tells
    /// `hears` what happens to it.
    pub(super) fn follow_account(&self, address: &str, hears: Hears) -> Hold {
        let user = address.to_ascii_lowercase();
        // Its orders with it, once a key of its own can ask for them.
        let with_orders = self.keys.account(&user).is_some();
        let hub = self.feeds.hub::<Followed, Live>();
        let lease = hub.lease(
            (user.clone(), with_orders),
            Live::default,
            |node| self.start(node, &user, with_orders),
            || {},
        );
        let live = lease.state();
        let id = live.next_hearer.get();
        live.next_hearer.set(id + 1);
        live.hearers.borrow_mut().push((id, hears));
        Hold { lease, id }
    }

    /// Finds the account `user` trades with and subscribes its topics for
    /// the feed `node`, which holds them. Lighter is asked again, ever less
    /// often, while it cannot say; an address with no account has nothing
    /// to follow.
    fn start(&self, node: &Rc<Node>, user: &str, with_orders: bool) {
        let reach = Reach {
            api: self.api.clone(),
            queries: self.queries.downgrade(),
            kept: Rc::downgrade(&self.kept),
            node: Rc::downgrade(node),
            user: user.to_owned(),
        };
        let socket = self.socket.downgrade();
        let task = TaskHandle::spawn(async move {
            let mut wait = FIRST_RETRY;
            let account = loop {
                match accounts_of(&reach.api, &reach.queries, &reach.user).await {
                    Ok(accounts) => break accounts.trading(),
                    Err(error) => {
                        log::warn!(
                            "Lighter realtime: could not find {}'s account ({}); asking again",
                            reach.user,
                            error.message
                        );
                        timer(wait).await;
                        wait = (wait * 2).min(LAST_RETRY);
                    }
                }
            };
            let (Some(account), Some(node), Some(socket)) =
                (account, reach.node.upgrade(), socket.upgrade())
            else {
                return;
            };
            let topic = |channel: Channel| Topic::account(channel, account, &reach.user);
            // Asked for on behalf of the feed's holds, as what a feed asks
            // for while it starts is.
            let (subscriptions, orders) = node.scope(|| {
                let positions = {
                    let (reach, opened) = (reach.clone(), Cell::new(false));
                    socket.subscribe(topic(Channel::Positions), move |frame: &Value| {
                        // What a subscription opens with is what a request
                        // answered; opened again after a gap, it is what
                        // may have changed meanwhile.
                        let first = is_snapshot(frame) && !opened.replace(true);
                        if !first && names_positions(frame) {
                            reach.refresh();
                        }
                    })
                };
                let trades = {
                    let reach = reach.clone();
                    socket.subscribe(topic(Channel::Trades), move |frame: &Value| {
                        if let Some(queries) = reach.queries.upgrade() {
                            reach.trades(&queries, account, frame);
                        }
                    })
                };
                let orders = with_orders.then(|| {
                    let reach = reach.clone();
                    socket.subscribe(topic(Channel::Orders), move |frame: &Value| {
                        if let Some(queries) = reach.queries.upgrade() {
                            reach.orders(&queries, frame);
                        }
                    })
                });
                (vec![positions, trades], orders)
            });
            let live = node.state();
            *live.subscriptions.borrow_mut() = subscriptions;
            if orders.is_some() {
                let renewed = reach.node.clone();
                *live.renewing.borrow_mut() = Some(TaskHandle::spawn(async move {
                    loop {
                        timer(RENEW_EVERY).await;
                        let Some(node) = renewed.upgrade() else {
                            return;
                        };
                        if let Some(orders) = &*node.state().orders.borrow() {
                            orders.resubscribe();
                        }
                    }
                }));
            }
            *live.orders.borrow_mut() = orders;
        });
        *node.state().starting.borrow_mut() = Some(task);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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
    fn new_trades_join_the_trades_newest_first_and_each_once() {
        let ids =
            |fills: &Fills| -> Vec<String> { fills.0.iter().map(|fill| fill.id.clone()).collect() };
        let known = Fills(vec![fill("3", 30.0), fill("2", 20.0), fill("1", 10.0)]);
        // One it holds and two it does not, one of them older than its
        // newest (what a subscription opens with after a gap).
        let arrived = [fill("5", 50.0), fill("3", 30.0), fill("4", 25.0)];
        let fills = with_new(&known, &arrived).unwrap();
        assert_eq!(ids(&fills), ["5", "3", "4", "2", "1"]);
        // Nothing new is nothing to give.
        assert!(with_new(&fills, &arrived).is_none());
        assert!(with_new(&known, &[]).is_none());
    }

    #[test]
    fn a_frame_of_positions_says_whether_any_changed() {
        // As the live socket sent them on 2026-11-22: all of them when
        // subscribed, and none with a funding round.
        let opened = json!({
            "channel": "account_all_positions:765951",
            "positions": { "1": { "market_id": 1, "symbol": "BTC", "position": "0.28190" } },
            "type": "subscribed/account_all_positions",
        });
        let funded = json!({
            "channel": "account_all_positions:765951",
            "last_funding_round": { "1": "0.243092" },
            "positions": {},
            "type": "update/account_all_positions",
        });
        assert!(names_positions(&opened) && is_snapshot(&opened));
        assert!(!names_positions(&funded) && !is_snapshot(&funded));
        assert!(!names_positions(
            &json!({ "type": "update/account_all_positions" })
        ));
    }
}
