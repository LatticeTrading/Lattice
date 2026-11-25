//! An account's open orders ([`Request::ActiveOrders`](super::Request), and
//! the stream of them: [`super::live`]), as Lighter writes them and as
//! trading reads them ([`OpenOrders`]): the newest first.
//!
//! - an order is known by its index (`order_id`, the same as text: it does
//!   not fit a float), which is what cancels it;
//! - `is_ask` says it sells; `remaining_base_amount` is what is left to
//!   fill; `price` is its limit, or for one that trades at the market's
//!   price once triggered the worst price it may trade at;
//! - `type` says what it is (`limit`, `stop-loss`, `take-profit-limit`),
//!   which reads as the names every exchange's orders go by here (`Limit`,
//!   `Stop Market`, `Take Profit Limit`); `trigger_price` is the price one
//!   waits for, `0` for one that waits for none;
//! - an order that only reduces and was placed with no size
//!   (`initial_base_amount` of zero) is the exit of a whole position: it
//!   takes the position's size when it triggers;
//! - the exits placed with an order wait for it to trade
//!   (`trigger_status` of `parent-order`) and name it
//!   (`parent_order_id`). They are read as its exits and not listed
//!   themselves until it trades, which is how every exchange's read here;
//!   one whose order is not among the orders is listed as what it is;
//! - `status` says where it stands. One that is `open`, `pending` (waiting
//!   for its trigger or its order) or `in-progress` is open; anything else
//!   (filled, cancelled for whatever reason) is over, which is how the
//!   stream says an order ended.
//!
//! The stream sends the orders that changed, by market. They are applied to
//! the orders last known ([`Active::apply`]): one that is over leaves, one
//! that is open takes its place or joins. What a request answers is all of
//! them, and takes their place whole.
//!
//! Members Lighter may add are ignored; one it leaves out reads as not
//! reported.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{
    markets::Directory,
    state::{figure, list},
};
use crate::data::trading::{Exits, OpenOrder, OpenOrders, Side};

/// The reply to [`Request::ActiveOrders`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct ActiveOrdersReply {
    #[serde(default, deserialize_with = "list")]
    orders: Vec<Order>,
}

/// Yes or no, as Lighter writes one: `true`, or `1` where an order is
/// written as its transaction was.
fn flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Bool(flag) => flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        _ => false,
    })
}

/// One order, as Lighter writes it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub(super) struct Order {
    order_index: u64,
    market_index: u64,
    market_symbol: String,
    #[serde(deserialize_with = "figure")]
    initial_base_amount: Option<f64>,
    #[serde(deserialize_with = "figure")]
    remaining_base_amount: Option<f64>,
    #[serde(deserialize_with = "figure")]
    price: Option<f64>,
    #[serde(deserialize_with = "flag")]
    is_ask: bool,
    #[serde(rename = "type")]
    kind: String,
    #[serde(deserialize_with = "flag")]
    reduce_only: bool,
    #[serde(deserialize_with = "figure")]
    trigger_price: Option<f64>,
    status: String,
    trigger_status: String,
    parent_order_index: u64,
    /// When it was placed: Unix seconds, or milliseconds.
    #[serde(deserialize_with = "figure")]
    timestamp: Option<f64>,
}

impl Order {
    /// Whether it still rests or waits.
    fn open(&self) -> bool {
        // One that says nothing of where it stands is taken to be listed
        // for being open.
        matches!(
            self.status.as_str(),
            "open" | "pending" | "in-progress" | ""
        )
    }

    /// Whether it is an exit that waits for the order it was placed with.
    fn waits_for_parent(&self) -> bool {
        self.parent_order_index != 0 && self.trigger_status == "parent-order"
    }

    /// The price it waits for, for one that does.
    fn trigger(&self) -> Option<f64> {
        self.trigger_price.filter(|price| *price > 0.0)
    }

    /// What its kind is called here.
    fn kind(&self) -> String {
        match self.kind.as_str() {
            "" | "limit" => "Limit",
            "market" => "Market",
            "stop-loss" => "Stop Market",
            "stop-loss-limit" => "Stop Limit",
            "take-profit" => "Take Profit Market",
            "take-profit-limit" => "Take Profit Limit",
            "twap" | "twap-sub" => "TWAP",
            other => return other.to_owned(),
        }
        .to_owned()
    }

    /// The order as trading lists it, with `exits` waiting for it and its
    /// market named `instrument`.
    fn listed(&self, instrument: String, exits: Exits) -> OpenOrder {
        let placed_with_none = self.initial_base_amount.is_some_and(|size| size == 0.0);
        OpenOrder {
            id: self.order_index.to_string(),
            instrument,
            side: if self.is_ask { Side::Sell } else { Side::Buy },
            price: self.price.filter(|price| *price > 0.0),
            size: self.remaining_base_amount,
            reduce_only: self.reduce_only,
            kind: self.kind(),
            trigger_price: self.trigger(),
            exits,
            whole_position: self.reduce_only && placed_with_none,
        }
    }
}

/// An account's open orders as Lighter last said them, in no order: what
/// the stream's changes are applied to, and what [`OpenOrders`] are read
/// from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Active(Vec<Order>);

impl From<ActiveOrdersReply> for Active {
    fn from(reply: ActiveOrdersReply) -> Self {
        Self(reply.orders.into_iter().filter(Order::open).collect())
    }
}

impl Active {
    /// These orders with `changed` applied: each of them that is over
    /// leaves, and each that is open takes the place of the one with its
    /// index, or joins.
    #[must_use]
    pub(super) fn apply(&self, changed: Vec<Order>) -> Self {
        let mut orders = self.0.clone();
        for order in changed {
            let known = orders
                .iter()
                .position(|known| known.order_index == order.order_index);
            match (known, order.open()) {
                (Some(place), true) => orders[place] = order,
                (Some(place), false) => {
                    orders.swap_remove(place);
                }
                (None, true) => orders.push(order),
                (None, false) => {}
            }
        }
        Self(orders)
    }

    /// The orders as trading lists them, the newest first, their markets
    /// named as `directory` knows them where they do not say.
    pub(super) fn listed(&self, directory: &Directory) -> OpenOrders {
        let present = |index: u64| self.0.iter().any(|order| order.order_index == index);
        // The exits that wait for an order here, by that order.
        let mut waiting: HashMap<u64, Exits> = HashMap::new();
        for exit in self.0.iter().filter(|order| order.waits_for_parent()) {
            if !present(exit.parent_order_index) {
                continue;
            }
            let exits = waiting.entry(exit.parent_order_index).or_default();
            if exit.kind.starts_with("take-profit") {
                exits.take_profit = exits.take_profit.or(exit.trigger());
            } else if exit.kind.starts_with("stop-loss") {
                exits.stop_loss = exits.stop_loss.or(exit.trigger());
            }
        }
        let mut shown: Vec<&Order> = self
            .0
            .iter()
            .filter(|order| !(order.waits_for_parent() && present(order.parent_order_index)))
            .collect();
        // The newest first; one that says no time last.
        shown.sort_by(|a, b| {
            let at = |order: &Order| order.timestamp.unwrap_or(f64::NEG_INFINITY);
            at(b)
                .total_cmp(&at(a))
                .then(b.order_index.cmp(&a.order_index))
        });
        let named = |order: &Order| {
            if !order.market_symbol.is_empty() {
                return order.market_symbol.clone();
            }
            directory
                .symbol(order.market_index)
                .unwrap_or_else(|| format!("#{}", order.market_index))
        };
        OpenOrders(
            shown
                .into_iter()
                .map(|order| {
                    let exits = waiting.remove(&order.order_index).unwrap_or_default();
                    order.listed(named(order), exits)
                })
                .collect(),
        )
    }
}

/// The orders a frame of the stream names: its `orders`, by market. `None`
/// for a frame that is none of theirs; orders that do not read are left
/// out, not the rest with them.
pub(super) fn changed(frame: &Value) -> Option<Vec<Order>> {
    let by_market = frame.get("orders")?.as_object()?;
    let orders = by_market
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|order| {
            Order::deserialize(order)
                .map_err(|error| log::warn!("Lighter realtime: unreadable order: {error}"))
                .ok()
        });
    Some(orders.collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// An order as Lighter's documentation writes one, with `changes` over
    /// it.
    fn order(index: u64, changes: Value) -> Value {
        let mut order = json!({
            "order_index": index, "client_order_index": 0, "order_id": index.to_string(),
            "client_order_id": "0", "market_index": 1, "owner_account_index": 2_180,
            "initial_base_amount": "0.00150", "price": "61125.7", "nonce": 643_418,
            "remaining_base_amount": "0.00100", "is_ask": false, "base_size": 150,
            "base_price": 611_257, "filled_base_amount": "0.00050",
            "filled_quote_amount": "30.562850", "side": "", "type": "limit",
            "time_in_force": "good-till-time", "reduce_only": false, "trigger_price": "0.0",
            "order_expiry": 1_797_761_008_933_u64, "status": "open", "trigger_status": "na",
            "trigger_time": 0, "parent_order_index": 0, "parent_order_id": "0",
            "to_trigger_order_id_0": "0", "to_trigger_order_id_1": "0",
            "to_cancel_order_id_0": "0", "block_height": 102_202,
            "timestamp": 1_766_387_932, "created_at": 1_766_387_932,
            "updated_at": 1_766_387_932, "market_symbol": "BTC", "market_kind": "perps",
        });
        for (name, value) in changes.as_object().expect("an object of changes") {
            order[name.as_str()] = value.clone();
        }
        order
    }

    fn active(orders: Vec<Value>) -> Active {
        let reply: ActiveOrdersReply =
            serde_json::from_value(json!({ "code": 200, "orders": orders })).unwrap();
        reply.into()
    }

    fn ids(orders: &OpenOrders) -> Vec<&str> {
        orders.0.iter().map(|order| order.id.as_str()).collect()
    }

    #[test]
    fn open_orders_read_newest_first_by_the_names_orders_go_by() {
        let directory = Directory::default();
        let orders = active(vec![
            order(281_474_977_354_074, json!({})),
            // A stop of a whole short, placed later.
            order(
                562_949_291_740_362,
                json!({ "type": "stop-loss", "is_ask": false, "reduce_only": true,
                        "initial_base_amount": "0.00000", "remaining_base_amount": "0.00000",
                        "price": "70557.3", "trigger_price": "64143.0", "status": "pending",
                        "trigger_status": "mark-price", "timestamp": 1_766_388_000,
                        "market_index": 0, "market_symbol": "ETH" }),
            ),
            // One that is over is not open, whatever a reply lists.
            order(7, json!({ "status": "canceled-margin-not-allowed" })),
        ])
        .listed(&directory);
        assert_eq!(ids(&orders), ["562949291740362", "281474977354074"]);

        let stop = &orders.0[0];
        assert_eq!((stop.instrument.as_str(), stop.side), ("ETH", Side::Buy));
        assert_eq!(stop.kind, "Stop Market");
        assert_eq!(stop.trigger_price, Some(64_143.0));
        assert_eq!((stop.price, stop.size), (Some(70_557.3), Some(0.0)));
        assert!(stop.reduce_only && stop.whole_position);

        let bid = &orders.0[1];
        assert_eq!((bid.instrument.as_str(), bid.side), ("BTC", Side::Buy));
        assert_eq!(bid.kind, "Limit");
        // What is left of it, and no trigger: `0.0` is none.
        assert_eq!((bid.price, bid.size), (Some(61_125.7), Some(0.001)));
        assert_eq!(bid.trigger_price, None);
        assert!(!bid.reduce_only && !bid.whole_position && bid.exits.is_empty());

        // Every kind by its name here; one that is new by its own.
        for (kind, named) in [
            ("market", "Market"),
            ("stop-loss-limit", "Stop Limit"),
            ("take-profit", "Take Profit Market"),
            ("take-profit-limit", "Take Profit Limit"),
            ("twap", "TWAP"),
            ("iceberg", "iceberg"),
        ] {
            let listed = active(vec![order(1, json!({ "type": kind }))]).listed(&directory);
            assert_eq!(listed.0[0].kind, named);
        }

        // What is left out is not reported, and a market that is not named
        // goes by what is known of it, or by its index.
        let bare = active(vec![json!({ "order_index": 9, "market_index": 4 })]);
        let listed = bare.listed(&directory);
        assert_eq!((listed.0[0].price, listed.0[0].size), (None, None));
        assert_eq!(
            (listed.0[0].instrument.as_str(), listed.0[0].kind.as_str()),
            ("#4", "Limit")
        );
        directory.read(&std::rc::Rc::new(super::super::markets::tests::markets()));
        assert_eq!(bare.listed(&directory).0[0].instrument, "1000PEPE");

        // Nothing open reads as nothing, not as a failure.
        let none: ActiveOrdersReply = serde_json::from_value(json!({ "code": 200 })).unwrap();
        assert_eq!(Active::from(none).listed(&directory), OpenOrders::default());
    }

    #[test]
    fn the_exits_placed_with_an_order_wait_for_it_as_its_own() {
        let directory = Directory::default();
        let exit = |index: u64, kind: &str, trigger: &str, parent: u64| {
            order(
                index,
                json!({ "type": kind, "is_ask": true, "reduce_only": true,
                        "initial_base_amount": "0.00000", "remaining_base_amount": "0.00000",
                        "trigger_price": trigger, "status": "pending",
                        "trigger_status": "parent-order", "parent_order_index": parent,
                        "parent_order_id": parent.to_string() }),
            )
        };
        let orders = active(vec![
            order(100, json!({})),
            exit(101, "take-profit", "65000.0", 100),
            exit(102, "stop-loss", "58000.0", 100),
        ]);
        // One order, with what closes what it opens.
        let listed = orders.listed(&directory);
        assert_eq!(ids(&listed), ["100"]);
        assert_eq!(
            listed.0[0].exits,
            Exits {
                take_profit: Some(65_000.0),
                stop_loss: Some(58_000.0),
            }
        );

        // Once the order has traded it is over, and its exits wait for
        // their prices: orders of their own, of the whole position.
        let traded = orders.apply(vec![
            serde_json::from_value(order(100, json!({ "status": "filled" }))).unwrap(),
            serde_json::from_value(exit(101, "take-profit", "65000.0", 100)).unwrap(),
        ]);
        let listed = traded.listed(&directory);
        assert_eq!(listed.0.len(), 2);
        assert!(listed.0.iter().all(|order| order.whole_position));
        let kinds: Vec<&str> = listed.0.iter().map(|order| order.kind.as_str()).collect();
        assert!(kinds.contains(&"Take Profit Market") && kinds.contains(&"Stop Market"));
    }

    #[test]
    fn the_stream_changes_the_orders_last_known() {
        let directory = Directory::default();
        let known = active(vec![
            order(1, json!({})),
            order(2, json!({ "timestamp": 5 })),
        ]);
        // As the stream sends them: by market, each as it stands now.
        let frame = json!({
            "channel": "account_all_orders:2180",
            "orders": {
                "1": [
                    order(1, json!({ "remaining_base_amount": "0.00020" })),
                    order(2, json!({ "status": "canceled" })),
                    order(4, json!({ "status": "filled" })),
                ],
                "0": [order(3, json!({ "market_symbol": "ETH", "timestamp": 1_766_390_000 }))],
                "7": "not orders",
            },
            "type": "update/account_all_orders",
        });
        let changed = changed(&frame).unwrap();
        assert_eq!(changed.len(), 4);
        let now = known.apply(changed).listed(&directory);
        // The one that joined is the newest; the cancelled one is gone, and
        // one that came and went between two frames never was.
        assert_eq!(ids(&now), ["3", "1"]);
        assert_eq!(now.0[1].size, Some(0.0002));
        // Applying nothing changes nothing, and a frame of something else
        // names no orders.
        assert_eq!(known.apply(Vec::new()), known);
        assert!(super::changed(&json!({ "type": "connected" })).is_none());
        assert_eq!(super::changed(&json!({ "orders": {} })), Some(Vec::new()));
    }
}
