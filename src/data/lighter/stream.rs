//! Lighter's WebSocket (`/stream`): JSON text frames, one subscription per
//! channel followed.
//!
//! Client frames:
//!
//! ```json
//! {"type":"subscribe","channel":"market_stats/1"}
//! {"type":"subscribe","channel":"account_all_orders/2180","auth":"…"}
//! {"type":"unsubscribe","channel":"market_stats/1"}
//! {"type":"ping"}
//! ```
//!
//! Server frames name a `type`: `connected` once, on opening; `pong`;
//! `subscribed/<channel>` with what the channel is of as it stands, then
//! `update/<channel>` as it changes, both naming their `channel` with a
//! colon where the subscription has a slash (`market_stats:1`);
//! `unsubscribed`, naming the channel the same way. A refusal has no type:
//! an `error` of a `code` and a `message`, which ends in the channel it is
//! about when it is about one (`Already Subscribed to : market_stats:1`).
//!
//! What is followed here ([`Topic`]):
//!
//! - a market's statistics (`market_stats/<market>`): its mark price, its
//!   best bid and offer and their middle, about once a second;
//! - an account's positions (`account_all_positions/<account>`): all of
//!   them when subscribed, then those that changed;
//! - an account's trades (`account_all_trades/<account>`): its latest when
//!   subscribed, then each as it happens;
//! - an account's orders (`account_all_orders/<account>`): those that
//!   changed, each as it stands now. Lighter sends them only to an auth
//!   token of the account's own, which the subscription carries
//!   ([`Keys::token_of`]): a new one each time the socket subscribes, the
//!   first time and after a reconnect.
//!
//! The server ends a connection it has heard nothing from for two minutes;
//! the shared socket's keepalive pings every twenty seconds, and each ping
//! is answered.
//!
//! The socket is opened as a reading one (`?readonly=true`): nothing is
//! sent over it but subscriptions (transactions are posted,
//! [`super::requests`]), and Lighter serves a reading socket where it serves
//! no other.
//!
//! Checked 2026-11-22 against the live socket: the greeting; the market
//! statistics' first frame and one a second after it (`mid_price`,
//! `best_bid_price`, `best_ask_price`, `mark_price`); an account's positions
//! (all of them, then `{}` with a funding round) and its trades, both
//! without a token; the refusals of a channel that is none, of a market that
//! is none (which names no channel), of a second subscribe, of an
//! unsubscribe of what is not subscribed, and of the orders without a token
//! (`auth field is required: account_all_orders:1`); the unsubscribe's
//! acknowledgement; a ping's pong; and the connection closed after 120
//! seconds of saying nothing, and kept for as long as it was pinged. The
//! orders' frames could not be: they take a registered key.

use std::rc::Rc;

use nsh_core::{time::unix_ms, websocket::WebSocketData};
use serde_json::{Value, json};

use super::{accounts::VENUE, key::Keys};
use crate::data::{
    net::{Inbound, Route, SocketProtocol, Subject},
    portfolio::short_address,
};

/// What of an account is followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Channel {
    /// Its positions: all of them, then those that change.
    Positions,
    /// Its trades: the latest, then each as it happens.
    Trades,
    /// Its orders, as each changes: only to a token of its own.
    Orders,
}

impl Channel {
    /// The channel's name, as subscriptions and frames write it.
    const fn name(self) -> &'static str {
        match self {
            Self::Positions => "account_all_positions",
            Self::Trades => "account_all_trades",
            Self::Orders => "account_all_orders",
        }
    }

    /// What the connection panel calls it.
    const fn label(self) -> &'static str {
        match self {
            Self::Positions => "Positions",
            Self::Trades => "Trades",
            Self::Orders => "Open orders",
        }
    }
}

/// The channel of a market's statistics.
const MARKET_STATS: &str = "market_stats";

/// Something followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Topic {
    /// One thing of one account.
    Account {
        channel: Channel,
        /// The account's index.
        account: u64,
        /// The address it is an account of, which is what it is called.
        owner: String,
    },
    /// The statistics of one market.
    Market {
        /// The market's index.
        id: u64,
        /// Its symbol, which is what it is called.
        symbol: String,
    },
}

impl Topic {
    /// `channel` of the account `account` of the address `owner`.
    pub(super) fn account(channel: Channel, account: u64, owner: &str) -> Self {
        Self::Account {
            channel,
            account,
            owner: owner.to_owned(),
        }
    }

    /// The statistics of the market `id`, named `symbol`.
    pub(super) fn market(id: u64, symbol: &str) -> Self {
        Self::Market {
            id,
            symbol: symbol.to_owned(),
        }
    }

    /// The channel's name and what of it is followed.
    fn parts(&self) -> (&'static str, u64) {
        match self {
            Self::Account {
                channel, account, ..
            } => (channel.name(), *account),
            Self::Market { id, .. } => (MARKET_STATS, *id),
        }
    }

    /// The channel as a subscription names it.
    fn subscription(&self) -> String {
        let (name, of) = self.parts();
        format!("{name}/{of}")
    }
}

/// The key of the channel `said` (a frame's `channel`) when it is one
/// followed, as [`SocketProtocol::key`] writes it; `None` otherwise.
fn key_of(said: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    let said = said.trim();
    known(said).then(|| said.to_owned())
}

/// The key of the channel a refusal is about: what its message ends in,
/// after the last colon that words come before.
fn key_in(message: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    let (_, said) = message.rsplit_once(": ")?;
    key_of(said, known)
}

/// Lighter's WebSocket protocol, with the keys its subscriptions are
/// authorized by.
pub(super) struct Protocol {
    pub(super) keys: Rc<Keys>,
}

impl Protocol {
    /// The token a subscription of `topic` carries: one of the account's
    /// key, for what is sent only to it.
    fn token(&self, topic: &Topic) -> Option<String> {
        let Topic::Account {
            channel: Channel::Orders,
            account,
            ..
        } = topic
        else {
            return None;
        };
        // Whole seconds since 1970 fit a `u64` for good.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let now = (unix_ms() / 1000.0).max(0.0) as u64;
        self.keys.token_of(*account, now)
    }
}

impl SocketProtocol for Protocol {
    type Topic = Topic;
    /// A data frame, whole: its `type` says whether it is what the channel
    /// is of as it stands or what changed.
    type Message = Value;

    fn key(&self, topic: &Topic) -> String {
        let (name, of) = topic.parts();
        format!("{name}:{of}")
    }

    fn channel<'a>(&self, topic: &'a Topic) -> &'a str {
        topic.parts().0
    }

    fn subscribe_frame(&self, topic: &Topic) -> String {
        let mut frame = json!({ "type": "subscribe", "channel": topic.subscription() });
        if let Some(token) = self.token(topic) {
            frame["auth"] = Value::String(token);
        }
        frame.to_string()
    }

    fn unsubscribe_frame(&self, topic: &Topic) -> String {
        json!({ "type": "unsubscribe", "channel": topic.subscription() }).to_string()
    }

    fn ping_frame(&self) -> Option<String> {
        Some(r#"{"type":"ping"}"#.to_owned())
    }

    fn decode(&self, frame: &WebSocketData, known: &dyn Fn(&str) -> bool) -> Inbound<Value> {
        let WebSocketData::Text(text) = frame else {
            return Inbound::Malformed;
        };
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
            return Inbound::Malformed;
        };
        if let Some(error) = frame.get("error") {
            let code = error
                .get("code")
                .map(ToString::to_string)
                .unwrap_or_default();
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unreadable error")
                .to_owned();
            let key = key_in(&message, known);
            return match (code.as_str(), key) {
                // Already made, or already gone: where it was to be.
                ("30003", Some(key)) => Inbound::Acknowledged(key),
                ("30002", Some(key)) => Inbound::Dropped(key),
                ("30002" | "30003", None) => Inbound::Ignored,
                (_, key) => Inbound::Issue {
                    fatal: key.is_some(),
                    key,
                    code,
                    message,
                },
            };
        }
        let kind = frame
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let channel = frame.get("channel").and_then(Value::as_str);
        match kind {
            "pong" => Inbound::Pong,
            // The greeting, and a ping of the server's own, which the pings
            // sent every few seconds answer as well as a pong would.
            "connected" | "ping" => Inbound::Ignored,
            "unsubscribed" => match channel.and_then(|said| key_of(said, known)) {
                Some(key) => Inbound::Dropped(key),
                None => Inbound::Ignored,
            },
            kind if kind.starts_with("subscribed/") || kind.starts_with("update/") => match channel
            {
                Some(said) => Inbound::Data {
                    route: Route::Topic(said.trim().to_owned()),
                    message: Value::Object(frame),
                },
                None => Inbound::Ignored,
            },
            "" => Inbound::Malformed,
            _ => Inbound::Ignored,
        }
    }

    fn subject(&self, topic: &Topic) -> Subject {
        let (name, detail) = match topic {
            Topic::Account { channel, owner, .. } => (short_address(owner), channel.label()),
            Topic::Market { symbol, .. } => (symbol.clone(), "Prices"),
        };
        Subject {
            exchange: VENUE.to_owned(),
            name,
            detail: detail.to_owned(),
        }
    }
}

/// Whether a data frame is what its channel is of as it stands (the first
/// one a subscription is answered with), not what changed since.
pub(super) fn is_snapshot(frame: &Value) -> bool {
    let kind = frame.get("type").and_then(Value::as_str);
    kind.is_some_and(|kind| kind.starts_with("subscribed/"))
}

#[cfg(test)]
mod tests {
    use super::{super::key::ApiKey, *};

    const OWNER: &str = "0xe117d3d94b30f7e7aa4f29704d0d2f43c009b9a8";

    fn protocol() -> Protocol {
        Protocol {
            keys: Rc::default(),
        }
    }

    fn decode(protocol: &Protocol, frame: &str) -> Inbound<Value> {
        let known = |key: &str| {
            [
                "market_stats:1",
                "account_all_orders:2180",
                "account_all_trades:2180",
            ]
            .contains(&key)
        };
        protocol.decode(&WebSocketData::Text(frame.to_owned()), &known)
    }

    #[test]
    fn a_channel_is_subscribed_by_its_name_and_what_it_is_of() {
        let protocol = protocol();
        let market = Topic::market(1, "BTC");
        assert_eq!(protocol.key(&market), "market_stats:1");
        assert_eq!(protocol.channel(&market), "market_stats");
        assert_eq!(
            protocol.subscribe_frame(&market),
            r#"{"channel":"market_stats/1","type":"subscribe"}"#
        );
        assert_eq!(
            protocol.unsubscribe_frame(&market),
            r#"{"channel":"market_stats/1","type":"unsubscribe"}"#
        );
        assert_eq!(protocol.ping_frame().as_deref(), Some(r#"{"type":"ping"}"#));

        let trades = Topic::account(Channel::Trades, 2_180, OWNER);
        assert_eq!(protocol.key(&trades), "account_all_trades:2180");
        assert_eq!(
            protocol.subscribe_frame(&trades),
            r#"{"channel":"account_all_trades/2180","type":"subscribe"}"#
        );
        let positions = Topic::account(Channel::Positions, 2_180, OWNER);
        assert_eq!(protocol.key(&positions), "account_all_positions:2180");

        // What the connection panel calls them.
        let subject = protocol.subject(&trades);
        assert_eq!(
            (subject.name.as_str(), subject.detail.as_str()),
            ("0xe117…b9a8", "Trades")
        );
        let subject = protocol.subject(&market);
        assert_eq!(
            (subject.name.as_str(), subject.detail.as_str()),
            ("BTC", "Prices")
        );
        assert_eq!(subject.exchange, "lighterxyz");
    }

    #[test]
    fn an_accounts_orders_are_subscribed_with_a_token_of_its_key() {
        let protocol = protocol();
        let orders = Topic::account(Channel::Orders, 2_180, OWNER);
        // No key: no token, and Lighter says what is missing.
        assert_eq!(
            protocol.subscribe_frame(&orders),
            r#"{"channel":"account_all_orders/2180","type":"subscribe"}"#
        );
        protocol
            .keys
            .hold(OWNER, ApiKey::random(2_180, 253).unwrap());
        let frame: Value = serde_json::from_str(&protocol.subscribe_frame(&orders)).unwrap();
        assert_eq!(frame["channel"], "account_all_orders/2180");
        let token = frame["auth"].as_str().expect("a token");
        let parts: Vec<&str> = token.split(':').collect();
        assert_eq!((parts.len(), parts[1], parts[2]), (4, "2180", "253"));
        assert_eq!(parts[3].len(), 160);
        // Only the orders carry one, and only the account's own.
        let trades = Topic::account(Channel::Trades, 2_180, OWNER);
        assert!(!protocol.subscribe_frame(&trades).contains("auth"));
        let other = Topic::account(Channel::Orders, 7, OWNER);
        assert!(!protocol.subscribe_frame(&other).contains("auth"));
        // Unsubscribing needs none.
        assert!(!protocol.unsubscribe_frame(&orders).contains("auth"));
    }

    #[test]
    fn frames_read_as_what_the_server_meant() {
        let protocol = protocol();
        // As the live socket sent them on 2026-11-22.
        assert!(matches!(
            decode(&protocol, r#"{"session_id":"5051d06f","type":"connected"}"#),
            Inbound::Ignored
        ));
        assert!(matches!(
            decode(&protocol, r#"{"type":"pong"}"#),
            Inbound::Pong
        ));
        assert!(matches!(
            decode(&protocol, r#"{"type":"ping"}"#),
            Inbound::Ignored
        ));
        // What a channel is of as it stands, then what changed: both for
        // its topic, whole.
        for (kind, snapshot) in [("subscribed", true), ("update", false)] {
            let frame = format!(
                r#"{{"channel":"market_stats:1","market_stats":{{"symbol":"BTC","market_id":1,"mark_price":"61260.8","mid_price":"61245.2"}},"type":"{kind}/market_stats"}}"#
            );
            let Inbound::Data {
                route: Route::Topic(key),
                message,
            } = decode(&protocol, &frame)
            else {
                panic!("a data frame");
            };
            assert_eq!(key, "market_stats:1");
            assert_eq!(message["market_stats"]["mark_price"], "61260.8");
            assert_eq!(is_snapshot(&message), snapshot);
        }
        assert!(matches!(
            decode(&protocol, r#"{"type": "unsubscribed", "channel": "market_stats:1"}"#),
            Inbound::Dropped(key) if key == "market_stats:1"
        ));
        // One for a channel nothing follows is nobody's.
        assert!(matches!(
            decode(
                &protocol,
                r#"{"type": "unsubscribed", "channel": "market_stats:2"}"#
            ),
            Inbound::Ignored
        ));

        // Refusals: of the orders without a token, which fails their topic;
        // of a channel that is none, which is nobody's.
        let refused = r#"{"error":{"code":20001,"message":"invalid param : auth field is required: account_all_orders:2180"}}"#;
        let Inbound::Issue {
            key,
            fatal,
            code,
            message,
        } = decode(&protocol, refused)
        else {
            panic!("an issue");
        };
        assert_eq!(key.as_deref(), Some("account_all_orders:2180"));
        assert!(fatal);
        assert_eq!(code, "20001");
        assert!(message.starts_with("invalid param"));
        assert!(matches!(
            decode(
                &protocol,
                r#"{"error":{"code":30005,"message":"Invalid Channel:  (marketId)"}}"#
            ),
            Inbound::Issue {
                key: None,
                fatal: false,
                ..
            }
        ));
        // One already made is made, and one already gone is gone.
        assert!(matches!(
            decode(&protocol, r#"{"error":{"code":30003,"message":"Already Subscribed to : market_stats:1"}}"#),
            Inbound::Acknowledged(key) if key == "market_stats:1"
        ));
        assert!(matches!(
            decode(&protocol, r#"{"error":{"code":30002,"message":"Not Subscribed to : market_stats:1"}}"#),
            Inbound::Dropped(key) if key == "market_stats:1"
        ));
        assert!(matches!(
            decode(
                &protocol,
                r#"{"error":{"code":30002,"message":"Not Subscribed to : market_stats:2"}}"#
            ),
            Inbound::Ignored
        ));

        // What is no frame of this protocol.
        for garbage in ["", "pong", "[]", "{}"] {
            assert!(
                matches!(decode(&protocol, garbage), Inbound::Malformed),
                "{garbage}"
            );
        }
        let binary = WebSocketData::Bytes(vec![1, 2, 3]);
        assert!(matches!(
            protocol.decode(&binary, &|_| true),
            Inbound::Malformed
        ));
    }
}
