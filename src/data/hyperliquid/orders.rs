//! A user's open orders ([`Info::FrontendOpenOrders`](super::Info)), as
//! Hyperliquid writes them and as trading reads them ([`OpenOrders`]):
//! the newest first.
//!
//! - `side` is `B` for a bid (a buy) and `A` for an ask (a sell);
//! - `sz` is what is left to fill (`origSz`, what was placed, is not
//!   read: nothing shows it);
//! - `triggerPx` is `0.0` for an order that waits for no trigger, which
//!   reads as none;
//! - `children` are the exits placed with an order, which wait for it to
//!   trade: each is an order itself, a take profit or a stop loss by its
//!   `orderType` (`Take Profit Market`, `Stop Market`), and what is read of
//!   it is its trigger. They are read leniently, one that is neither being
//!   left out: the order is listed whatever its exits look like;
//! - `isPositionTpsl` marks the exit of a whole position, whose `sz` is
//!   `0.0`: it takes the position's size when it triggers.
//!
//! Members Hyperliquid may add are ignored; one it leaves out reads as not
//! reported.

use serde::Deserialize;
use serde_json::Value;

use crate::data::{
    net::Figure,
    trading::{Exits, OpenOrder, OpenOrders, Side},
};

/// The reply to [`Info::FrontendOpenOrders`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct FrontendOpenOrders(Vec<Order>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Order {
    coin: String,
    side: String,
    limit_px: Option<Figure>,
    sz: Option<Figure>,
    oid: u64,
    /// When it was placed, in Unix milliseconds.
    timestamp: Option<f64>,
    #[serde(default)]
    reduce_only: bool,
    /// `Limit`, `Stop Market`, `Take Profit Limit`.
    order_type: Option<String>,
    #[serde(default)]
    is_trigger: bool,
    trigger_px: Option<Figure>,
    #[serde(default)]
    is_position_tpsl: bool,
    #[serde(default)]
    children: Vec<Value>,
}

/// The exits among `children`: the trigger of the first take profit and of
/// the first stop loss.
fn exits_of(children: &[Value]) -> Exits {
    let trigger = |named: &str| {
        children.iter().find_map(|child| {
            let kind = child.get("orderType")?.as_str()?;
            let Figure(trigger) = Figure::of(child.get("triggerPx")?)?;
            (kind.starts_with(named) && trigger > 0.0).then_some(trigger)
        })
    };
    Exits {
        take_profit: trigger("Take Profit"),
        stop_loss: trigger("Stop"),
    }
}

impl From<FrontendOpenOrders> for OpenOrders {
    fn from(FrontendOpenOrders(mut placed): FrontendOpenOrders) -> Self {
        // The newest first; one that says no time last.
        placed.sort_by(|a, b| {
            let at = |order: &Order| order.timestamp.unwrap_or(f64::NEG_INFINITY);
            at(b).total_cmp(&at(a))
        });
        let orders = placed
            .into_iter()
            .map(|order| OpenOrder {
                id: order.oid.to_string(),
                instrument: order.coin,
                side: if order.side == "A" {
                    Side::Sell
                } else {
                    Side::Buy
                },
                price: order.limit_px.map(|price| price.0),
                size: order.sz.map(|size| size.0),
                reduce_only: order.reduce_only,
                kind: order.order_type.unwrap_or_else(|| "Limit".to_owned()),
                trigger_price: order
                    .trigger_px
                    .map(|price| price.0)
                    .filter(|price| order.is_trigger && *price > 0.0),
                exits: exits_of(&order.children),
                whole_position: order.is_position_tpsl,
            })
            .collect();
        Self(orders)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn open_orders_read_newest_first() {
        // The shapes of Hyperliquid's documentation.
        let reply: FrontendOpenOrders = serde_json::from_value(json!([
            {
                "coin": "BTC", "isPositionTpsl": false, "isTrigger": false,
                "limitPx": "29792.0", "oid": 91_490_942, "orderType": "Limit",
                "origSz": "5.0", "reduceOnly": false, "side": "A", "sz": "4.5",
                "timestamp": 1_681_247_412_573_u64, "triggerCondition": "N/A",
                "triggerPx": "0.0"
            },
            {
                "coin": "ETH", "isTrigger": true, "limitPx": "1500.0", "oid": 91_490_943,
                "orderType": "Stop Market", "origSz": "1.0", "reduceOnly": true,
                "side": "B", "sz": "1.0", "timestamp": 1_681_247_500_000_u64,
                "triggerPx": "1510.0"
            },
            { "coin": "SOL", "side": "B", "oid": 7 },
        ]))
        .unwrap();
        let orders = OpenOrders::from(reply).0;
        let listed: Vec<&str> = orders
            .iter()
            .map(|order| order.instrument.as_str())
            .collect();
        assert_eq!(listed, ["ETH", "BTC", "SOL"]);

        let stop = &orders[0];
        assert_eq!((stop.side, stop.reduce_only), (Side::Buy, true));
        assert_eq!(stop.kind, "Stop Market");
        assert_eq!(stop.trigger_price, Some(1510.0));

        let ask = &orders[1];
        assert_eq!(ask.id, "91490942");
        assert_eq!(ask.side, Side::Sell);
        assert_eq!((ask.price, ask.size), (Some(29_792.0), Some(4.5)));
        // No trigger: `0.0` is none.
        assert_eq!(ask.trigger_price, None);

        // What is left out is not reported.
        let bare = &orders[2];
        assert_eq!((bare.price, bare.size), (None, None));
        assert_eq!(bare.kind, "Limit");
        assert!(bare.exits.is_empty() && !bare.whole_position);
    }

    #[test]
    fn exits_read_from_an_orders_children_and_a_positions_own() {
        let reply: FrontendOpenOrders = serde_json::from_value(json!([
            // An order with both exits waiting for it, and a child that is
            // neither.
            {
                "coin": "BTC", "side": "B", "limitPx": "61000.0", "sz": "0.01", "oid": 1,
                "orderType": "Limit", "origSz": "0.01", "timestamp": 3,
                "children": [
                    { "coin": "BTC", "side": "A", "limitPx": "58500.0", "sz": "0.01", "oid": 2,
                      "isTrigger": true, "triggerPx": "65000.0", "reduceOnly": true,
                      "orderType": "Take Profit Market", "children": [] },
                    { "coin": "BTC", "side": "A", "limitPx": "52200.0", "sz": "0.01", "oid": 3,
                      "isTrigger": true, "triggerPx": "58000.0", "reduceOnly": true,
                      "orderType": "Stop Market", "children": [] },
                    { "unexpected": true },
                    7
                ]
            },
            // A position's stop, as Hyperliquid listed one on 2026-11-22.
            {
                "coin": "BTC", "side": "B", "limitPx": "70557.0", "sz": "0.0", "oid": 4,
                "timestamp": 2, "triggerCondition": "Price above 64143", "isTrigger": true,
                "triggerPx": "64143.0", "children": [], "isPositionTpsl": true,
                "reduceOnly": true, "orderType": "Stop Market", "origSz": "0.0", "tif": null,
                "cloid": null
            },
            // Children of another shape altogether: the order still lists.
            { "coin": "ETH", "side": "A", "oid": 5, "timestamp": 1, "children": [[], "x"] },
        ]))
        .unwrap();
        let orders = OpenOrders::from(reply).0;
        assert_eq!(orders.len(), 3);
        assert_eq!(
            orders[0].exits,
            Exits {
                take_profit: Some(65_000.0),
                stop_loss: Some(58_000.0),
            }
        );
        assert!(!orders[0].whole_position);

        let stop = &orders[1];
        assert!(stop.whole_position && stop.reduce_only);
        assert_eq!(stop.kind, "Stop Market");
        assert_eq!((stop.size, stop.trigger_price), (Some(0.0), Some(64_143.0)));
        assert!(stop.exits.is_empty());
        assert!(orders[2].exits.is_empty());
    }
}
