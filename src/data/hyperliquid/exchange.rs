//! The exchange endpoint: `POST /exchange` with a signed action, which is
//! everything that changes an account (the info endpoint only reads).
//!
//! - The body is the action, the nonce its signature was made with, the
//!   signature, and the vault it is for (none here): [`Api::exchange`].
//! - An action weighs 1 of the address's limit ([`super::WEIGHT_LIMIT`]),
//!   and 1 more for every 40 orders or cancels in it; every action here
//!   holds three at most. It is spent from the client's budget as an info request's
//!   weight is, and a `429` handled the same way.
//! - The reply says `ok` with what the action made, or `err` with why not
//!   in words, under a 200 either way; an order or a cancel that the
//!   exchange took and then refused says so in its own status
//!   ([`placed`], [`cancelled`]). Both come back as an [`ApiError`] whose
//!   code is `REJECTED` and whose message is Hyperliquid's.
//!
//! The actions are made here as Hyperliquid's SDK makes them, member for
//! member ([`orders_action`], [`cancel_action`], [`leverage_action`]):
//! what is hashed for the signature is their MessagePack in that order
//! ([`Packed`]). The kept tests sign the SDK's own orders with the SDK's
//! key and get the SDK's signatures.
//!
//! An order is a limit order or a trigger order ([`WireKind`]). A trigger
//! order waits for the mark price to reach its trigger and then trades: at
//! the market's price when it is a market one, no worse than its limit
//! price. A take profit and a stop loss are trigger orders that only
//! reduce, and several orders of one action are grouped ([`Grouping`]): an
//! order with the exits that wait for it to trade, or the exits of a whole
//! position, which are sent with no size and take the position's.

use alloy::primitives::{Address, Signature};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    Api,
    pack::Packed,
    signing::{self, Network},
};
use crate::data::{
    net::{ApiError, Figure},
    trading::{Placed, TimeInForce},
};

/// The endpoint every action is posted to.
const PATH: &str = "/exchange";
/// What an action of one order or cancel costs of the address's limit.
const WEIGHT: u32 = 1;
/// The code of what Hyperliquid answered and would not do.
const REJECTED: &str = "REJECTED";

/// What is posted: an action and what authorizes it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Posted<'a> {
    action: &'a Packed,
    nonce: u64,
    signature: Value,
    /// The vault or sub-account the action is for; the account itself
    /// while there is none.
    vault_address: Option<String>,
}

/// The reply's envelope.
#[derive(Deserialize)]
struct Reply {
    status: String,
    #[serde(default)]
    response: Value,
}

/// What Hyperliquid said it would not do, as an error.
fn rejected(said: &Value) -> ApiError {
    let said = match said {
        Value::String(said) => said.clone(),
        other => other.to_string(),
    };
    ApiError::api(Some(200), REJECTED, said)
}

impl Api {
    /// Posts `action` with the `signature` made of it at `nonce`, once the
    /// budget has room, and gives what Hyperliquid made of it (the reply's
    /// `response`).
    pub(super) async fn exchange(
        &self,
        action: &Packed,
        nonce: u64,
        signature: &Signature,
    ) -> Result<Value, ApiError> {
        self.budget.spend(WEIGHT).await?;
        let posted = Posted {
            action,
            nonce,
            signature: signing::wire(signature),
            vault_address: None,
        };
        let reply: Reply = self.limited(self.http.post(PATH, &posted).await)?;
        if reply.status == "ok" {
            Ok(reply.response)
        } else {
            Err(rejected(&reply.response))
        }
    }
}

/// `value` as an action writes a price or a size: decimal text of at most
/// eight places, without the zeros after them. `None` for a figure eight
/// places do not hold, which would be sent as another one (the SDK's
/// `float_to_wire` refuses the same).
pub(super) fn decimal(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let written = format!("{value:.8}");
    let read: f64 = written.parse().ok()?;
    if (read - value).abs() >= 1e-12 {
        return None;
    }
    let written = match written.split_once('.') {
        Some((whole, places)) => {
            let places = places.trim_end_matches('0');
            if places.is_empty() {
                whole.to_owned()
            } else {
                format!("{whole}.{places}")
            }
        }
        None => written,
    };
    Some(if written == "-0" {
        "0".to_owned()
    } else {
        written
    })
}

/// Which exit a trigger order is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Exit {
    TakeProfit,
    StopLoss,
}

/// What kind of order an action lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WireKind {
    /// It trades at its price or better, for as long as this says.
    Limit(TimeInForce),
    /// It waits for the mark price to reach `trigger` ([`decimal`]), then
    /// trades at the market's price (`market`) or rests at its own.
    Trigger {
        trigger: String,
        market: bool,
        exit: Exit,
    },
}

/// An order as an action lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WireOrder {
    /// The asset's index in the universe ([`super::meta`]).
    pub asset: u32,
    pub buy: bool,
    /// The limit price ([`decimal`]): of a market trigger order, the worst
    /// it may trade at once triggered.
    pub price: String,
    /// `0` for the exit of a whole position ([`Grouping::Position`]).
    pub size: String,
    pub reduce_only: bool,
    pub kind: WireKind,
}

/// How the orders of one action belong together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grouping {
    /// Each for itself.
    Apart,
    /// An order, then the exits that wait for it to trade and go with it.
    WithExits,
    /// The exits of the position held, which take its size.
    PositionExits,
}

/// What Hyperliquid calls a time in force.
fn tif(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::GoodTilCancelled => "Gtc",
        TimeInForce::ImmediateOrCancel => "Ioc",
        // "Add liquidity only".
        TimeInForce::PostOnly => "Alo",
    }
}

/// `order` as an action lists it (the SDK's `order_request_to_order_wire`,
/// its type as `order_type_to_wire` writes it).
fn wire(order: &WireOrder) -> Packed {
    let kind = match &order.kind {
        WireKind::Limit(time_in_force) => (
            "limit",
            Packed::Map(vec![("tif", Packed::text(tif(*time_in_force)))]),
        ),
        WireKind::Trigger {
            trigger,
            market,
            exit,
        } => (
            "trigger",
            Packed::Map(vec![
                ("isMarket", Packed::Bool(*market)),
                ("triggerPx", Packed::text(trigger)),
                (
                    "tpsl",
                    Packed::text(match exit {
                        Exit::TakeProfit => "tp",
                        Exit::StopLoss => "sl",
                    }),
                ),
            ]),
        ),
    };
    Packed::Map(vec![
        ("a", Packed::Uint(order.asset.into())),
        ("b", Packed::Bool(order.buy)),
        ("p", Packed::text(&order.price)),
        ("s", Packed::text(&order.size)),
        ("r", Packed::Bool(order.reduce_only)),
        ("t", Packed::Map(vec![kind])),
    ])
}

/// The action that places `orders`, grouped as `grouping` says (the SDK's
/// `order_wires_to_order_action`).
pub(super) fn orders_action(orders: &[WireOrder], grouping: Grouping) -> Packed {
    let grouping = match grouping {
        Grouping::Apart => "na",
        Grouping::WithExits => "normalTpsl",
        Grouping::PositionExits => "positionTpsl",
    };
    Packed::Map(vec![
        ("type", Packed::text("order")),
        ("orders", Packed::List(orders.iter().map(wire).collect())),
        ("grouping", Packed::text(grouping)),
    ])
}

/// The action that sets how the account trades the asset `asset`: its
/// leverage, and whether its margin is the account's (`cross`) or the
/// position's own (the SDK's `update_leverage`).
pub(super) fn leverage_action(asset: u32, cross: bool, leverage: u32) -> Packed {
    Packed::Map(vec![
        ("type", Packed::text("updateLeverage")),
        ("asset", Packed::Uint(asset.into())),
        ("isCross", Packed::Bool(cross)),
        ("leverage", Packed::Uint(leverage.into())),
    ])
}

/// The action that cancels the order `order` of the asset `asset`.
pub(super) fn cancel_action(asset: u32, order: u64) -> Packed {
    let cancel = Packed::Map(vec![
        ("a", Packed::Uint(asset.into())),
        ("o", Packed::Uint(order)),
    ]);
    Packed::Map(vec![
        ("type", Packed::text("cancel")),
        ("cancels", Packed::List(vec![cancel])),
    ])
}

/// The action that approves `agent`, as it is posted beside the wallet's
/// signature of [`signing::approve_agent`] with the same members. It is
/// user-signed: what was signed is the typed data, and this only tells
/// Hyperliquid what that was (`signatureChainId` is the chain the wallet
/// signed on).
pub(super) fn approve_agent_action(
    network: Network,
    chain_id: u64,
    agent: Address,
    name: &str,
    nonce: u64,
) -> Packed {
    Packed::Map(vec![
        ("type", Packed::text("approveAgent")),
        ("hyperliquidChain", Packed::text(network.name())),
        ("signatureChainId", Packed::text(format!("{chain_id:#x}"))),
        ("agentAddress", Packed::text(format!("{agent:#x}"))),
        ("agentName", Packed::text(name)),
        ("nonce", Packed::Uint(nonce)),
    ])
}

/// The statuses of an action's `response`: what became of each of its
/// orders or cancels, in their order.
fn statuses(response: &Value) -> Result<&[Value], ApiError> {
    response
        .pointer("/data/statuses")
        .and_then(Value::as_array)
        .filter(|statuses| !statuses.is_empty())
        .map(Vec::as_slice)
        .ok_or_else(|| ApiError::invalid_response("the exchange's reply names no order"))
}

/// The first status of an action's `response`: what became of its one
/// order or cancel, or of the first of several.
fn first_status(response: &Value) -> Result<&Value, ApiError> {
    statuses(response).map(|statuses| &statuses[0])
}

/// What became of the order an `order` action placed, from the action's
/// `response`: of the first one, when exits were placed with it. An exit
/// that was refused is an error, whatever became of the order itself (the
/// account's open orders, asked for after every action, say that).
pub(super) fn placed(response: &Value) -> Result<Placed, ApiError> {
    let status = first_status(response)?;
    let id = |made: &Value| made.get("oid").map(ToString::to_string).unwrap_or_default();
    if let Some(refused) = status.get("error") {
        return Err(rejected(refused));
    }
    exits_placed(response)?;
    if let Some(resting) = status.get("resting") {
        return Ok(Placed::Resting { id: id(resting) });
    }
    if let Some(filled) = status.get("filled") {
        let figure = |name: &str| filled.get(name).and_then(Figure::of).map(|figure| figure.0);
        return Ok(match (figure("totalSz"), figure("avgPx")) {
            (Some(size), Some(price)) => Placed::Filled {
                id: id(filled),
                size,
                price,
            },
            _ => Placed::Accepted,
        });
    }
    // `waitingForFill`, `waitingForTrigger`, and whatever comes later.
    Ok(Placed::Accepted)
}

/// Whether every order of an `order` action was taken, from the action's
/// `response`: what is asked of a position's exits, none of which is the
/// one that matters more.
pub(super) fn exits_placed(response: &Value) -> Result<(), ApiError> {
    match statuses(response)?
        .iter()
        .find_map(|status| status.get("error"))
    {
        Some(refused) => Err(rejected(refused)),
        None => Ok(()),
    }
}

/// Whether a `cancel` action cancelled its order, from the action's
/// `response`.
pub(super) fn cancelled(response: &Value) -> Result<(), ApiError> {
    match first_status(response)?.get("error") {
        Some(refused) => Err(rejected(refused)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::{address, b256};
    use serde_json::json;

    use super::{
        super::signing::tests::{expected, sdk_signature},
        *,
    };

    /// The SDK's test order: 100 ETH (asset 1) bought at 100.
    fn sdk_order(time_in_force: TimeInForce) -> WireOrder {
        WireOrder {
            asset: 1,
            buy: true,
            price: decimal(100.0).unwrap(),
            size: decimal(100.0).unwrap(),
            reduce_only: false,
            kind: WireKind::Limit(time_in_force),
        }
    }

    /// The action of one order, each for itself.
    fn order_action(order: &WireOrder) -> Packed {
        orders_action(std::slice::from_ref(order), Grouping::Apart)
    }

    /// `test_l1_action_signing_order_matches` of hyperliquid-python-sdk
    /// (`tests/signing_test.py`): the SDK's order, key and nonce give the
    /// SDK's signatures on both nets, so the action is hashed and signed
    /// as Hyperliquid verifies it.
    #[test]
    fn an_order_is_signed_as_hyperliquids_sdk_signs_it() {
        let action = order_action(&sdk_order(TimeInForce::GoodTilCancelled));
        assert_eq!(
            serde_json::to_string(&action).unwrap(),
            r#"{"type":"order","orders":[{"a":1,"b":true,"p":"100","s":"100","r":false,"t":{"limit":{"tif":"Gtc"}}}],"grouping":"na"}"#
        );
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Mainnet),
            expected(
                "0xd65369825a9df5d80099e513cce430311d7d26ddf477f5b3a33d2806b100d78e",
                "0x2b54116ff64054968aa237c20ca9ff68000f977c93289157748a3162b6ea940e",
                28
            )
        );
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Testnet),
            expected(
                "0x82b2ba28e76b3d761093aaded1b1cdad4960b3af30212b343fb2e6cdfa4e3d54",
                "0x6b53878fc99d26047f4d7e8c90eb98955a109f44209163f52d8dc4278cbbd9f5",
                27
            )
        );
    }

    /// `test_phantom_agent_creation_matches_production`: an order as the
    /// production site once sent it hashes to the connection id it signed.
    #[test]
    fn an_order_hashes_to_the_connection_id_production_signed() {
        let action = order_action(&WireOrder {
            asset: 4,
            buy: true,
            price: decimal(1670.1).unwrap(),
            size: decimal(0.0147).unwrap(),
            reduce_only: false,
            kind: WireKind::Limit(TimeInForce::ImmediateOrCancel),
        });
        assert_eq!(
            signing::action_hash(&action, 1_677_777_606_040, None),
            b256!("0x0fcbeda5ae3c4950a548021552a4fea2226858c4453571bf3f24ba017eac2908")
        );
    }

    /// `test_l1_action_signing_tpsl_order_matches`: the SDK's stop loss (a
    /// market trigger order at 103) gives the SDK's signatures on both
    /// nets, so a trigger order is written and hashed as Hyperliquid
    /// verifies it.
    #[test]
    fn a_trigger_order_is_signed_as_hyperliquids_sdk_signs_it() {
        let stop = WireOrder {
            kind: WireKind::Trigger {
                trigger: decimal(103.0).unwrap(),
                market: true,
                exit: Exit::StopLoss,
            },
            ..sdk_order(TimeInForce::GoodTilCancelled)
        };
        let action = order_action(&stop);
        assert_eq!(
            serde_json::to_string(&action).unwrap(),
            r#"{"type":"order","orders":[{"a":1,"b":true,"p":"100","s":"100","r":false,"t":{"trigger":{"isMarket":true,"triggerPx":"103","tpsl":"sl"}}}],"grouping":"na"}"#
        );
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Mainnet),
            expected(
                "0x98343f2b5ae8e26bb2587daad3863bc70d8792b09af1841b6fdd530a2065a3f9",
                "0x6b5bb6bb0633b710aa22b721dd9dee6d083646a5f8e581a20b545be6c1feb405",
                27
            )
        );
        assert_eq!(
            sdk_signature(&action, 0, None, Network::Testnet),
            expected(
                "0x971c554d917c44e0e1b6cc45d8f9404f32172a9d3b3566262347d0302896a2e4",
                "0x206257b104788f80450f8e786c329daa589aa0b32ba96948201ae556d5637eac",
                28
            )
        );
    }

    #[test]
    fn exits_are_grouped_with_their_order_or_their_position() {
        let exit = |exit: Exit, trigger: f64, price: f64, size: &str| WireOrder {
            asset: 0,
            buy: false,
            price: decimal(price).unwrap(),
            size: size.to_owned(),
            reduce_only: true,
            kind: WireKind::Trigger {
                trigger: decimal(trigger).unwrap(),
                market: true,
                exit,
            },
        };
        let entry = WireOrder {
            asset: 0,
            buy: true,
            price: decimal(61_000.0).unwrap(),
            size: "0.01".to_owned(),
            reduce_only: false,
            kind: WireKind::Limit(TimeInForce::GoodTilCancelled),
        };
        // An order, then its take profit and its stop loss, as the SDK's
        // `basic_tpsl.py` sends them.
        let with_exits = orders_action(
            &[
                entry,
                exit(Exit::TakeProfit, 65_000.0, 58_500.0, "0.01"),
                exit(Exit::StopLoss, 58_000.0, 52_200.0, "0.01"),
            ],
            Grouping::WithExits,
        );
        assert_eq!(
            serde_json::to_string(&with_exits).unwrap(),
            concat!(
                r#"{"type":"order","orders":["#,
                r#"{"a":0,"b":true,"p":"61000","s":"0.01","r":false,"t":{"limit":{"tif":"Gtc"}}},"#,
                r#"{"a":0,"b":false,"p":"58500","s":"0.01","r":true,"t":{"trigger":{"isMarket":true,"triggerPx":"65000","tpsl":"tp"}}},"#,
                r#"{"a":0,"b":false,"p":"52200","s":"0.01","r":true,"t":{"trigger":{"isMarket":true,"triggerPx":"58000","tpsl":"sl"}}}"#,
                r#"],"grouping":"normalTpsl"}"#
            )
        );
        // A position's exits have no size of their own.
        let of_position = orders_action(
            &[exit(Exit::StopLoss, 58_000.0, 52_200.0, "0")],
            Grouping::PositionExits,
        );
        assert_eq!(
            serde_json::to_string(&of_position).unwrap(),
            r#"{"type":"order","orders":[{"a":0,"b":false,"p":"52200","s":"0","r":true,"t":{"trigger":{"isMarket":true,"triggerPx":"58000","tpsl":"sl"}}}],"grouping":"positionTpsl"}"#
        );
    }

    #[test]
    fn figures_are_written_as_the_sdk_writes_them() {
        for (value, written) in [
            (100.0, "100"),
            (1670.1, "1670.1"),
            (0.0147, "0.0147"),
            (0.000_012_31, "0.00001231"),
            (61_000.5, "61000.5"),
            (0.0, "0"),
            (-0.0, "0"),
        ] {
            assert_eq!(decimal(value).as_deref(), Some(written), "{value}");
        }
        // What eight places do not hold is not sent as something else.
        assert_eq!(decimal(0.000_012_312_312), None);
        assert_eq!(decimal(f64::NAN), None);
        assert_eq!(decimal(f64::INFINITY), None);
    }

    #[test]
    fn cancels_and_approvals_are_written_member_for_member() {
        assert_eq!(
            serde_json::to_string(&cancel_action(4, 91_490_942)).unwrap(),
            r#"{"type":"cancel","cancels":[{"a":4,"o":91490942}]}"#
        );
        // The SDK's `update_leverage`: its members in its order.
        assert_eq!(
            serde_json::to_string(&leverage_action(1, false, 21)).unwrap(),
            r#"{"type":"updateLeverage","asset":1,"isCross":false,"leverage":21}"#
        );
        let agent = address!("0x5E9ee1089755c3435139848e47e6635505d5A13a");
        assert_eq!(
            serde_json::to_value(approve_agent_action(
                Network::Mainnet,
                42_161,
                agent,
                "Lattice",
                1_700_000_000_000
            ))
            .unwrap(),
            json!({
                "type": "approveAgent",
                "hyperliquidChain": "Mainnet",
                "signatureChainId": "0xa4b1",
                "agentAddress": "0x5e9ee1089755c3435139848e47e6635505d5a13a",
                "agentName": "Lattice",
                "nonce": 1_700_000_000_000_u64,
            })
        );
    }

    #[test]
    fn an_orders_status_says_what_became_of_it() {
        let response = |status: Value| json!({ "type": "order", "data": { "statuses": [status] } });
        assert_eq!(
            placed(&response(json!({ "resting": { "oid": 77_738_308 } }))),
            Ok(Placed::Resting {
                id: "77738308".to_owned()
            })
        );
        assert_eq!(
            placed(&response(json!({
                "filled": { "totalSz": "0.02", "avgPx": "1891.4", "oid": 77_747_314 }
            }))),
            Ok(Placed::Filled {
                id: "77747314".to_owned(),
                size: 0.02,
                price: 1891.4
            })
        );
        assert_eq!(
            placed(&response(json!("waitingForTrigger"))),
            Ok(Placed::Accepted)
        );
        // Taken and then refused: Hyperliquid's words, as an error.
        let refused = placed(&response(json!({
            "error": "Order must have minimum value of $10."
        })))
        .unwrap_err();
        assert_eq!(refused.code, REJECTED);
        assert_eq!(refused.message, "Order must have minimum value of $10.");
        assert!(placed(&json!({ "type": "order" })).is_err());

        // An order with exits answers for each: the order's is what became
        // of it, and an exit that was refused is said, whatever the order's.
        let group = |statuses: Value| json!({ "type": "order", "data": { "statuses": statuses } });
        assert_eq!(
            placed(&group(json!([
                { "resting": { "oid": 9 } },
                "waitingForFill",
                "waitingForFill"
            ]))),
            Ok(Placed::Resting { id: "9".to_owned() })
        );
        let exit_refused = group(json!([
            { "resting": { "oid": 9 } },
            { "error": "Invalid TP/SL price." },
            "waitingForFill"
        ]));
        assert_eq!(
            placed(&exit_refused).unwrap_err().message,
            "Invalid TP/SL price."
        );
        // A position's exits are all there is to answer for.
        assert_eq!(
            exits_placed(&group(json!(["waitingForTrigger", "waitingForTrigger"]))),
            Ok(())
        );
        assert_eq!(
            exits_placed(&exit_refused).unwrap_err().message,
            "Invalid TP/SL price."
        );
        assert!(exits_placed(&group(json!([]))).is_err());

        let response =
            |status: Value| json!({ "type": "cancel", "data": { "statuses": [status] } });
        assert_eq!(cancelled(&response(json!("success"))), Ok(()));
        let gone = cancelled(&response(json!({
            "error": "Order was never placed, already canceled, or filled."
        })))
        .unwrap_err();
        assert_eq!(
            gone.message,
            "Order was never placed, already canceled, or filled."
        );
    }
}
