//! Hyperliquid's WebSocket (`/ws`): JSON text frames, one subscription per
//! thing followed.
//!
//! Client frames:
//!
//! ```json
//! {"method":"subscribe","subscription":{"type":"openOrders","user":"0x…"}}
//! {"method":"unsubscribe","subscription":{…the same…}}
//! {"method":"ping"}
//! ```
//!
//! Server frames name a `channel`: `pong`; `subscriptionResponse`, whose
//! `data` echoes the `method` and the `subscription` (with what was left
//! out filled in: `"dex":""`); `error`, whose `data` is text ending in the
//! request or subscription it is about (`Already subscribed: {…}`); and
//! the data channels, named as their subscription's `type`, whose `data`
//! names the `user` it is of.
//!
//! What is followed here ([`Topic`]) is an account, or an asset:
//!
//! - an account ([`Channel`]): the three subscriptions whose data names
//!   its user, so a frame routes to exactly one topic. (`orderUpdates` and
//!   `userEvents` do not name theirs, and `openOrders` carries what the
//!   first would be applied to.) Each sends the whole of what it is of
//!   when subscribed and again as it changes, but for the fills, which
//!   send the latest as a snapshot and then each new one;
//! - an asset's context (`activeAssetCtx`, by its `coin`): what it is
//!   marked at (`markPx`), with its mid and oracle prices and its funding,
//!   once when subscribed and then about once a second. Its data names the
//!   coin. A coin Hyperliquid does not list is refused (`Invalid
//!   subscription {…}`), which fails its topic;
//! - an asset's best bid and offer (`bbo`, by its `coin`): both as they
//!   stand when subscribed, then again each time either changes, which is
//!   several times a second for a busy asset. Its data names the coin. A
//!   delisted asset's is acknowledged and says nothing. **A coin
//!   Hyperliquid does not know is not refused: the connection is closed**,
//!   and with it every other topic, again on each reconnect while the
//!   topic is wanted. So nothing subscribes one for a coin whose context
//!   has not spoken first ([`super::pricing`]).
//!
//! The server ends a connection it hears nothing from for a minute; the
//! shared socket's keepalive pings every twenty seconds.
//!
//! Checked 2026-11-22 against the live socket: the three subscriptions'
//! acknowledgements and first frames for an account with orders and one
//! with nothing, a repeated subscribe, one that is no request, and the
//! unsubscribe's acknowledgement. The asset's context the same day: a
//! frame each second or so for `BTC` and `kPEPE` (14 in 14 seconds), for
//! a builder-deployed dex's asset (`xyz:XYZ100`) and for a delisted one
//! (`MATIC`, whose `midPx` is `null`); the acknowledgement echoing the
//! subscription as sent; and the refusal of a coin that is none. The best
//! bid and offer the same day: first frame within 0.1s of the
//! acknowledgement; over 15 seconds 97 frames for `BTC` (a median 115ms
//! apart) and 41 for `kPEPE` (181ms), against 16 of each one's context;
//! `xyz:XYZ100` the same way; `MATIC` acknowledged with no frame; and the
//! connection closed within 0.2s of a subscribe for `NOPE`, and for
//! `KPEPE` (the case of a name is part of it).

use nsh_core::websocket::WebSocketData;
use serde_json::{Value, json};

use super::accounts::VENUE;
use crate::data::{
    net::{Inbound, Route, SocketProtocol, Subject},
    portfolio::short_address,
};

/// What of an account is followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Channel {
    /// Its open orders: all of them, each time one changes.
    OpenOrders,
    /// Its perpetuals account: all of it, as it changes.
    ClearinghouseState,
    /// Its fills: the latest as a snapshot, then each as it happens.
    UserFills,
}

impl Channel {
    /// The subscription's `type`, and the channel its frames name.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::OpenOrders => "openOrders",
            Self::ClearinghouseState => "clearinghouseState",
            Self::UserFills => "userFills",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [Self::OpenOrders, Self::ClearinghouseState, Self::UserFills]
            .into_iter()
            .find(|channel| channel.name() == name)
    }

    /// What the connection panel calls it.
    const fn label(self) -> &'static str {
        match self {
            Self::OpenOrders => "Open orders",
            Self::ClearinghouseState => "Perpetuals account",
            Self::UserFills => "Trades",
        }
    }
}

/// What of an asset is followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum AssetFeed {
    /// Its context: what it is marked at, about once a second.
    Context,
    /// Its best bid and offer, each time either changes.
    Book,
}

impl AssetFeed {
    /// The subscription's `type`, and the channel its frames name.
    const fn name(self) -> &'static str {
        match self {
            Self::Context => "activeAssetCtx",
            Self::Book => "bbo",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [Self::Context, Self::Book]
            .into_iter()
            .find(|feed| feed.name() == name)
    }

    /// What the connection panel calls it.
    const fn label(self) -> &'static str {
        match self {
            Self::Context => "Mark price",
            Self::Book => "Best bid and offer",
        }
    }
}

/// Something followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Topic {
    /// One thing of one account.
    Account {
        channel: Channel,
        /// The account's address, in lower case.
        user: String,
    },
    /// One thing of one asset.
    Asset {
        feed: AssetFeed,
        /// The asset's name as Hyperliquid writes it (`BTC`, `kPEPE`: the
        /// case tells assets apart).
        coin: String,
    },
}

impl Topic {
    /// `channel` of the account at `user`.
    pub(super) fn new(channel: Channel, user: &str) -> Self {
        Self::Account {
            channel,
            user: user.to_ascii_lowercase(),
        }
    }

    /// `feed` of the asset `coin`.
    pub(super) fn asset(feed: AssetFeed, coin: &str) -> Self {
        Self::Asset {
            feed,
            coin: coin.to_owned(),
        }
    }

    /// The subscription's `type`, and the channel its frames name.
    fn name(&self) -> &'static str {
        match self {
            Self::Account { channel, .. } => channel.name(),
            Self::Asset { feed, .. } => feed.name(),
        }
    }

    /// The subscription, as both the subscribe and the unsubscribe name it.
    fn subscription(&self) -> Value {
        match self {
            Self::Account { channel, user } => {
                let mut subscription = json!({ "type": channel.name(), "user": user });
                if *channel == Channel::UserFills {
                    // What one order filled at one time is one fill, as the
                    // request for an account's trades asks for them.
                    subscription["aggregateByTime"] = Value::Bool(true);
                }
                subscription
            }
            Self::Asset { feed, coin } => json!({ "type": feed.name(), "coin": coin }),
        }
    }
}

/// The key of the topic on `channel` of `user`.
fn key(channel: &str, user: &str) -> String {
    format!("{channel}:{}", user.to_ascii_lowercase())
}

/// The key of `feed` of the asset `coin`, its name as it is written.
fn asset_key(feed: AssetFeed, coin: &str) -> String {
    format!("{}:{coin}", feed.name())
}

/// The key of the topic a `subscription` (as the server echoes one) is;
/// `None` for one that is none of these: another dex's, or neither an
/// account's nor an asset's.
fn key_of(subscription: &Value) -> Option<String> {
    let kind = subscription.get("type")?.as_str()?;
    if let Some(feed) = AssetFeed::named(kind) {
        return Some(asset_key(feed, subscription.get("coin")?.as_str()?));
    }
    let channel = Channel::named(kind)?;
    let user = subscription.get("user")?.as_str()?;
    let dex = subscription.get("dex").and_then(Value::as_str);
    dex.is_none_or(str::is_empty)
        .then(|| key(channel.name(), user))
}

/// The key of the topic an error's text is about: the text ends in the
/// subscription, or in the whole request, it could not take.
fn key_in(said: &str) -> Option<String> {
    let quoted: Value = serde_json::from_str(&said[said.find('{')?..]).ok()?;
    key_of(quoted.get("subscription").unwrap_or(&quoted))
}

/// Hyperliquid's WebSocket protocol.
pub(super) struct Protocol;

impl SocketProtocol for Protocol {
    type Topic = Topic;
    /// A data frame's `data`.
    type Message = Value;

    fn key(&self, topic: &Topic) -> String {
        match topic {
            Topic::Account { channel, user } => key(channel.name(), user),
            Topic::Asset { feed, coin } => asset_key(*feed, coin),
        }
    }

    fn channel<'a>(&self, topic: &'a Topic) -> &'a str {
        topic.name()
    }

    fn subscribe_frame(&self, topic: &Topic) -> String {
        json!({ "method": "subscribe", "subscription": topic.subscription() }).to_string()
    }

    fn unsubscribe_frame(&self, topic: &Topic) -> String {
        json!({ "method": "unsubscribe", "subscription": topic.subscription() }).to_string()
    }

    fn ping_frame(&self) -> Option<String> {
        Some(r#"{"method":"ping"}"#.to_owned())
    }

    fn decode(&self, frame: &WebSocketData, _known: &dyn Fn(&str) -> bool) -> Inbound<Value> {
        let WebSocketData::Text(text) = frame else {
            return Inbound::Malformed;
        };
        let Ok(Value::Object(mut frame)) = serde_json::from_str::<Value>(text) else {
            // The greeting older servers open with is plain text.
            return if text.starts_with("Websocket connection established") {
                Inbound::Ignored
            } else {
                Inbound::Malformed
            };
        };
        let Some(Value::String(channel)) = frame.remove("channel") else {
            return Inbound::Malformed;
        };
        let data = frame.remove("data").unwrap_or_default();
        match channel.as_str() {
            "pong" => Inbound::Pong,
            "subscriptionResponse" => {
                let Some(key) = data.get("subscription").and_then(key_of) else {
                    return Inbound::Ignored;
                };
                match data.get("method").and_then(Value::as_str) {
                    Some("subscribe") => Inbound::Acknowledged(key),
                    Some("unsubscribe") => Inbound::Dropped(key),
                    _ => Inbound::Ignored,
                }
            }
            "error" => {
                let message = data.as_str().unwrap_or("unreadable error").to_owned();
                // One already made or already gone is no failure of it.
                let again = message.starts_with("Already");
                let key = key_in(&message);
                Inbound::Issue {
                    fatal: key.is_some() && !again,
                    key,
                    code: "WS_ERROR".to_owned(),
                    message,
                }
            }
            name => {
                let named = |member: &str| data.get(member).and_then(Value::as_str);
                let key = if let Some(feed) = AssetFeed::named(name) {
                    named("coin").map(|coin| asset_key(feed, coin))
                } else {
                    let account = Channel::named(name).and_then(|_| named("user"));
                    account.map(|user| key(name, user))
                };
                match key {
                    Some(key) => Inbound::Data {
                        route: Route::Topic(key),
                        message: data,
                    },
                    // A channel nothing here subscribes, or one of its
                    // frames that names neither an account nor an asset.
                    None => Inbound::Ignored,
                }
            }
        }
    }

    fn subject(&self, topic: &Topic) -> Subject {
        let (name, detail) = match topic {
            Topic::Account { channel, user } => (short_address(user), channel.label()),
            Topic::Asset { feed, coin } => (coin.clone(), feed.label()),
        };
        Subject {
            exchange: VENUE.to_owned(),
            name,
            detail: detail.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "0xa62b923a112d50d03e1e096bbd53422490dac104";

    fn decoded(frame: &str) -> Inbound<Value> {
        Protocol.decode(&WebSocketData::Text(frame.to_owned()), &|_| true)
    }

    #[test]
    fn topics_are_subscribed_as_hyperliquid_reads_them() {
        let orders = Topic::new(
            Channel::OpenOrders,
            &USER.to_uppercase().replace("0X", "0x"),
        );
        assert_eq!(Protocol.key(&orders), format!("openOrders:{USER}"));
        assert_eq!(Protocol.channel(&orders), "openOrders");
        assert_eq!(
            Protocol.subscribe_frame(&orders),
            format!(
                r#"{{"method":"subscribe","subscription":{{"type":"openOrders","user":"{USER}"}}}}"#
            )
        );
        assert_eq!(
            Protocol.unsubscribe_frame(&orders),
            format!(
                r#"{{"method":"unsubscribe","subscription":{{"type":"openOrders","user":"{USER}"}}}}"#
            )
        );
        // The fills as the request for them asks: one per order and time.
        let fills = Topic::new(Channel::UserFills, USER);
        assert_eq!(
            Protocol.subscribe_frame(&fills),
            format!(
                r#"{{"method":"subscribe","subscription":{{"aggregateByTime":true,"type":"userFills","user":"{USER}"}}}}"#
            )
        );
        assert_eq!(
            Protocol.ping_frame().as_deref(),
            Some(r#"{"method":"ping"}"#)
        );
        let subject = Protocol.subject(&fills);
        assert_eq!(
            (subject.name.as_str(), subject.detail.as_str()),
            ("0xa62b…c104", "Trades")
        );
    }

    /// Frames as the live socket sent them on 2026-11-22.
    #[test]
    fn frames_read_as_what_they_are() {
        assert!(matches!(decoded(r#"{"channel":"pong"}"#), Inbound::Pong));

        // An acknowledgement names what was left out of the subscription.
        let subscribed = format!(
            r#"{{"channel":"subscriptionResponse","data":{{"method":"subscribe","subscription":{{"type":"openOrders","dex":"","user":"{USER}"}}}}}}"#
        );
        assert!(matches!(
            decoded(&subscribed),
            Inbound::Acknowledged(key) if key == format!("openOrders:{USER}")
        ));
        let unsubscribed = format!(
            r#"{{"channel":"subscriptionResponse","data":{{"method":"unsubscribe","subscription":{{"type":"userFills","user":"{USER}","aggregateByTime":true}}}}}}"#
        );
        assert!(matches!(
            decoded(&unsubscribed),
            Inbound::Dropped(key) if key == format!("userFills:{USER}")
        ));

        // Data routes by its channel and the account it names.
        let orders = format!(
            r#"{{"channel":"openOrders","data":{{"dex":"","user":"{USER}","orders":[]}}}}"#
        );
        match decoded(&orders) {
            Inbound::Data {
                route: Route::Topic(key),
                message,
            } => {
                assert_eq!(key, format!("openOrders:{USER}"));
                assert_eq!(message["orders"], json!([]));
            }
            _ => panic!("the open orders are data"),
        }
        // A channel nothing here subscribes reaches nobody, and is no
        // failure either.
        assert!(matches!(
            decoded(r#"{"channel":"orderUpdates","data":[]}"#),
            Inbound::Ignored
        ));
        assert!(matches!(
            decoded("Websocket connection established."),
            Inbound::Ignored
        ));
        for no_frame in ["", "[]", "{}", r#"{"data":1}"#, "pong"] {
            assert!(
                matches!(decoded(no_frame), Inbound::Malformed),
                "{no_frame}"
            );
        }
        assert!(matches!(
            Protocol.decode(&WebSocketData::Bytes(vec![1]), &|_| true),
            Inbound::Malformed
        ));
    }

    /// Frames as the live socket sent them on 2026-11-22.
    #[test]
    fn an_assets_context_is_a_topic_by_its_coin_as_written() {
        let pepe = Topic::asset(AssetFeed::Context, "kPEPE");
        assert_eq!(Protocol.key(&pepe), "activeAssetCtx:kPEPE");
        assert_eq!(Protocol.channel(&pepe), "activeAssetCtx");
        assert_eq!(
            Protocol.subscribe_frame(&pepe),
            r#"{"method":"subscribe","subscription":{"coin":"kPEPE","type":"activeAssetCtx"}}"#
        );
        assert_eq!(
            Protocol.unsubscribe_frame(&pepe),
            r#"{"method":"unsubscribe","subscription":{"coin":"kPEPE","type":"activeAssetCtx"}}"#
        );
        let subject = Protocol.subject(&pepe);
        assert_eq!(
            (subject.name.as_str(), subject.detail.as_str()),
            ("kPEPE", "Mark price")
        );

        let subscribed = r#"{"channel":"subscriptionResponse","data":{"method":"subscribe","subscription":{"type":"activeAssetCtx","coin":"kPEPE"}}}"#;
        assert!(matches!(
            decoded(subscribed),
            Inbound::Acknowledged(key) if key == "activeAssetCtx:kPEPE"
        ));
        // Its data names the coin, and routes by it.
        let context = r#"{"channel":"activeAssetCtx","data":{"coin":"kPEPE","ctx":{"funding":"-0.0000158407","openInterest":"4171721322.0","prevDayPx":"0.003372","dayNtlVlm":"12306618.63","premium":"-0.0005945303","oraclePx":"0.003364","markPx":"0.003361","midPx":"0.003361","impactPxs":["0.00336","0.003362"],"dayBaseVlm":"3680294027.0"}}}"#;
        match decoded(context) {
            Inbound::Data {
                route: Route::Topic(key),
                message,
            } => {
                assert_eq!(key, "activeAssetCtx:kPEPE");
                assert_eq!(message["ctx"]["markPx"], "0.003361");
            }
            _ => panic!("an asset's context is data"),
        }
        // One that names no coin reaches nobody.
        assert!(matches!(
            decoded(r#"{"channel":"activeAssetCtx","data":{"ctx":{}}}"#),
            Inbound::Ignored
        ));
        // A coin that is none is refused: its topic failed.
        let refused = r#"{"channel":"error","data":"Invalid subscription {\"type\":\"activeAssetCtx\",\"coin\":\"NOPE\"}"}"#;
        assert!(matches!(
            decoded(refused),
            Inbound::Issue { key: Some(key), fatal: true, .. } if key == "activeAssetCtx:NOPE"
        ));
    }

    /// Frames as the live socket sent them on 2026-11-22.
    #[test]
    fn an_assets_best_bid_and_offer_are_a_topic_of_their_own() {
        let book = Topic::asset(AssetFeed::Book, "kPEPE");
        assert_eq!(Protocol.key(&book), "bbo:kPEPE");
        assert_eq!(Protocol.channel(&book), "bbo");
        assert_eq!(
            Protocol.subscribe_frame(&book),
            r#"{"method":"subscribe","subscription":{"coin":"kPEPE","type":"bbo"}}"#
        );
        assert_eq!(
            Protocol.unsubscribe_frame(&book),
            r#"{"method":"unsubscribe","subscription":{"coin":"kPEPE","type":"bbo"}}"#
        );
        let subject = Protocol.subject(&book);
        assert_eq!(
            (subject.name.as_str(), subject.detail.as_str()),
            ("kPEPE", "Best bid and offer")
        );
        // One asset's two topics are told apart.
        let context = Topic::asset(AssetFeed::Context, "kPEPE");
        assert_ne!(Protocol.key(&book), Protocol.key(&context));

        let subscribed = r#"{"channel":"subscriptionResponse","data":{"method":"subscribe","subscription":{"type":"bbo","coin":"kPEPE"}}}"#;
        assert!(matches!(
            decoded(subscribed),
            Inbound::Acknowledged(key) if key == "bbo:kPEPE"
        ));
        let quoted = r#"{"channel":"bbo","data":{"coin":"kPEPE","time":1795323617730,"bbo":[{"px":"0.003341","sz":"9655883.0","n":16},{"px":"0.003342","sz":"37402.0","n":2}]}}"#;
        match decoded(quoted) {
            Inbound::Data {
                route: Route::Topic(key),
                message,
            } => {
                assert_eq!(key, "bbo:kPEPE");
                assert_eq!(message["bbo"][1]["px"], "0.003342");
            }
            _ => panic!("a best bid and offer is data"),
        }
    }

    #[test]
    fn errors_name_the_topic_they_are_about() {
        // One that is no request: its topic failed.
        let refused = format!(
            r#"{{"channel":"error","data":"Error parsing JSON into valid websocket request: {{\"method\":\"subscribe\",\"subscription\":{{\"type\":\"openOrders\",\"user\":\"{USER}\"}}}}"}}"#
        );
        match decoded(&refused) {
            Inbound::Issue {
                key, fatal, code, ..
            } => {
                assert_eq!(key, Some(format!("openOrders:{USER}")));
                assert!(fatal);
                assert_eq!(code, "WS_ERROR");
            }
            _ => panic!("an error is an issue"),
        }
        // One made twice is still made.
        let twice = format!(
            r#"{{"channel":"error","data":"Already subscribed: {{\"type\":\"userFills\",\"user\":\"{USER}\",\"aggregateByTime\":true}}"}}"#
        );
        assert!(matches!(
            decoded(&twice),
            Inbound::Issue { key: Some(key), fatal: false, .. } if key == format!("userFills:{USER}")
        ));
        // What names no topic is the connection's.
        assert!(matches!(
            decoded(r#"{"channel":"error","data":"Too many subscriptions"}"#),
            Inbound::Issue {
                key: None,
                fatal: false,
                ..
            }
        ));
    }
}
