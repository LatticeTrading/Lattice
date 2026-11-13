//! The Ferris WebSocket protocol (`/v1/ws`): JSON text frames.
//!
//! Client frames:
//!
//! ```json
//! {"op":"subscribe","channel":"trades","exchange":"binance","symbol":"BTC/USDT","params":{}}
//! {"op":"subscribe","channel":"marketstats","exchange":"binance","params":{},"marketIds":null,"fields":["funding"]}
//! {"op":"unsubscribe", ...the same members...}
//! {"op":"ping"}
//! ```
//!
//! Server frames carry a `type`: `pong`; `subscribed` / `alreadySubscribed`
//! / `unsubscribed` with the `topic`; `warning` / `error` with `code`,
//! `message` and, when about one, the `topic`; `marketstats` envelopes
//! (snapshot or delta of a statistics projection). Market data frames name
//! their `channel` and echo the `topic` with the `data`.
//!
//! A frame is routed by the key of the topic it echoes, computed exactly as
//! for the subscription: exchange lower-cased, symbol as requested, params
//! canonical. A symbol alias (the server echoing `BTC/USDT:USDT` for a
//! `BTC/USDT` request) therefore does not route: repair aliases where the
//! market is selected, never by loosening routing here. The socket does
//! report it: the unrouted key, and the acknowledged topic of the same
//! channel and exchange it most likely belongs to, are in its status.

use serde_json::{Map, Value, json};

use nsh_core::websocket::WebSocketData;

use crate::data::net::{ApiError, Inbound, Route, SocketProtocol, canonical_json};

/// A market data channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Trades,
    OrderBook,
    Ohlcv,
    /// Rejected by the deployed backend today (no ticker capability); kept
    /// so capability checks can name it.
    Ticker,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trades => "trades",
            Self::OrderBook => "orderbook",
            Self::Ohlcv => "ohlcv",
            Self::Ticker => "ticker",
        }
    }

    pub const ALL: [Self; 4] = [Self::Trades, Self::OrderBook, Self::Ohlcv, Self::Ticker];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str() == name)
    }
}

/// The statistics channel, whose topics are scopes rather than symbols.
const MARKET_STATS: &str = "marketstats";

/// What can be subscribed to.
#[derive(Clone, Debug, PartialEq)]
pub enum FerrisTopic {
    /// One market's channel. Every param that changes what the server sends
    /// (book `levels`, candle `timeframe`, `category`) belongs in `params`.
    Market {
        channel: Channel,
        exchange: String,
        symbol: String,
        params: Map<String, Value>,
    },
    /// A statistics projection: an exchange scope's markets (all of them, or
    /// the selected ids) and fields.
    MarketStats {
        exchange: String,
        params: Map<String, Value>,
        market_ids: Option<Vec<String>>,
        fields: Vec<String>,
    },
}

impl FerrisTopic {
    /// `channel` of `symbol` on `exchange`; `params` is an object or null.
    pub fn market(
        channel: Channel,
        exchange: &str,
        symbol: &str,
        params: Value,
    ) -> Result<Self, ApiError> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(ApiError::invalid_request("a Ferris topic needs a symbol"));
        }
        Ok(Self::Market {
            channel,
            exchange: exchange_id(exchange)?,
            symbol: symbol.to_owned(),
            params: params_object(params)?,
        })
    }

    /// The statistics projection of `fields` for the `market_ids` (none:
    /// every market) of `exchange`'s scope `params`. Ids and fields are
    /// sorted and deduplicated, as the server keys its projections.
    pub fn market_stats(
        exchange: &str,
        params: Value,
        market_ids: impl IntoIterator<Item = String>,
        fields: impl IntoIterator<Item = String>,
    ) -> Result<Self, ApiError> {
        let fields = sorted(fields);
        if fields.is_empty() {
            return Err(ApiError::invalid_request("a statistics topic needs fields"));
        }
        let market_ids = sorted(market_ids);
        Ok(Self::MarketStats {
            exchange: exchange_id(exchange)?,
            params: params_object(params)?,
            // The server rejects an empty list; omitting it means all markets.
            market_ids: (!market_ids.is_empty()).then_some(market_ids),
            fields,
        })
    }

    pub fn exchange(&self) -> &str {
        match self {
            Self::Market { exchange, .. } | Self::MarketStats { exchange, .. } => exchange,
        }
    }

    pub fn channel(&self) -> &str {
        match self {
            Self::Market { channel, .. } => channel.as_str(),
            Self::MarketStats { .. } => MARKET_STATS,
        }
    }

    /// The channel and exchange of a [`Self::key`] (`trades:binance`): what
    /// an alias of the same market keeps while its symbol or params differ.
    /// Neither contains a `:` (channels are fixed names, exchanges ids),
    /// so they are the key's first two fields.
    pub fn scope_of(key: &str) -> Option<&str> {
        let (end, _) = key.match_indices(':').nth(1)?;
        Some(&key[..end])
    }

    /// The identity two equal subscriptions share.
    pub fn key(&self) -> String {
        match self {
            Self::Market {
                channel,
                exchange,
                symbol,
                params,
            } => format!(
                "{}:{exchange}:{symbol}:{}",
                channel.as_str(),
                canonical_json(&Value::Object(params.clone()))
            ),
            Self::MarketStats {
                exchange,
                params,
                market_ids,
                fields,
            } => format!(
                "{MARKET_STATS}:{exchange}:{}:{}:{}",
                canonical_json(&Value::Object(params.clone())),
                json!(market_ids),
                json!(fields)
            ),
        }
    }

    fn frame(&self, op: &str) -> String {
        match self {
            Self::Market {
                channel,
                exchange,
                symbol,
                params,
            } => json!({
                "op": op,
                "channel": channel.as_str(),
                "exchange": exchange,
                "symbol": symbol,
                "params": params,
            }),
            Self::MarketStats {
                exchange,
                params,
                market_ids,
                fields,
            } => json!({
                "op": op,
                "channel": MARKET_STATS,
                "exchange": exchange,
                "params": params,
                "marketIds": market_ids,
                "fields": fields,
            }),
        }
        .to_string()
    }
}

fn exchange_id(exchange: &str) -> Result<String, ApiError> {
    let id = exchange.trim().to_ascii_lowercase();
    if id.is_empty() {
        return Err(ApiError::invalid_request(
            "a Ferris topic needs an exchange",
        ));
    }
    Ok(id)
}

fn params_object(params: Value) -> Result<Map<String, Value>, ApiError> {
    match params {
        Value::Null => Ok(Map::new()),
        Value::Object(params) => Ok(params),
        _ => Err(ApiError::invalid_request("Ferris params must be an object")),
    }
}

fn sorted(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut items: Vec<String> = items.into_iter().collect();
    items.sort_unstable();
    items.dedup();
    items
}

/// One data frame as every subscriber of its topic receives it.
#[derive(Clone, Debug)]
pub struct FerrisPush {
    channel: String,
    envelope: Value,
}

impl FerrisPush {
    /// `trades`, `orderbook`, `ohlcv` or `marketstats`.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// The payload of a market data frame (`data`); null when absent.
    pub fn data(&self) -> &Value {
        self.envelope.get("data").unwrap_or(&Value::Null)
    }

    /// The whole frame. A statistics envelope's members (`mode`,
    /// `generation`, `revision`, `previousRevision`, `markets`, `updates`,
    /// `removedMarketIds`) are read from here.
    pub fn envelope(&self) -> &Value {
        &self.envelope
    }
}

/// The Ferris wire format for [`crate::data::net::SharedSocket`].
pub struct FerrisProtocol;

impl SocketProtocol for FerrisProtocol {
    type Topic = FerrisTopic;
    type Message = FerrisPush;

    fn key(&self, topic: &FerrisTopic) -> String {
        topic.key()
    }

    fn channel<'a>(&self, topic: &'a FerrisTopic) -> &'a str {
        topic.channel()
    }

    fn subscribe_frame(&self, topic: &FerrisTopic) -> String {
        topic.frame("subscribe")
    }

    fn unsubscribe_frame(&self, topic: &FerrisTopic) -> String {
        topic.frame("unsubscribe")
    }

    fn ping_frame(&self) -> Option<String> {
        Some(r#"{"op":"ping"}"#.to_owned())
    }

    fn alias_scope<'k>(&self, key: &'k str) -> Option<&'k str> {
        FerrisTopic::scope_of(key)
    }

    fn decode(&self, frame: &WebSocketData, known: &dyn Fn(&str) -> bool) -> Inbound<FerrisPush> {
        let WebSocketData::Text(text) = frame else {
            return Inbound::Malformed;
        };
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
            return Inbound::Malformed;
        };
        let text_of = |name: &str| {
            frame
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let kind = frame.get("type").and_then(Value::as_str);
        // A data frame names its channel in `type` (`"type":"trades"`).
        let channel = frame
            .get("channel")
            .and_then(Value::as_str)
            .or(kind)
            .and_then(Channel::parse);
        let key = incoming_key(&frame, channel, known);
        match kind {
            Some("pong") => Inbound::Pong,
            Some("subscribed" | "alreadySubscribed") => {
                key.map_or(Inbound::Ignored, Inbound::Acknowledged)
            }
            Some("unsubscribed") => key.map_or(Inbound::Ignored, Inbound::Dropped),
            Some(kind @ ("warning" | "error")) => Inbound::Issue {
                key,
                fatal: kind == "error",
                code: text_of("code"),
                message: text_of("message"),
            },
            Some(MARKET_STATS) => data(MARKET_STATS, key, frame),
            _ => match channel {
                Some(channel) => data(channel.as_str(), key, frame),
                None => Inbound::Ignored,
            },
        }
    }
}

fn data(channel: &str, key: Option<String>, frame: Map<String, Value>) -> Inbound<FerrisPush> {
    Inbound::Data {
        route: key.map_or_else(|| Route::Channel(channel.to_owned()), Route::Topic),
        message: FerrisPush {
            channel: channel.to_owned(),
            envelope: Value::Object(frame),
        },
    }
}

/// The key of the topic a server frame echoes, normalized as subscriptions
/// are; `None` when it echoes none (or one that is not valid). A market
/// topic in a frame without a channel (acknowledgements, issues) is the one
/// `known` topic on any channel with its exchange, symbol and params, when
/// exactly one is.
fn incoming_key(
    frame: &Map<String, Value>,
    channel: Option<Channel>,
    known: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let topic = frame.get("topic")?.as_object()?;
    let exchange = topic.get("exchange")?.as_str()?;
    let params = topic.get("params").cloned().unwrap_or(Value::Null);
    // Statistics topics carry fields and never a symbol.
    if let Some(fields) = topic.get("fields").and_then(Value::as_array)
        && !topic.contains_key("symbol")
    {
        let strings = |items: &Vec<Value>| -> Option<Vec<String>> {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect()
        };
        let market_ids = match topic.get("marketIds") {
            None | Some(Value::Null) => Vec::new(),
            Some(ids) => strings(ids.as_array()?)?,
        };
        return FerrisTopic::market_stats(exchange, params, market_ids, strings(fields)?)
            .ok()
            .map(|topic| topic.key());
    }
    let symbol = topic.get("symbol")?.as_str()?;
    let key_on = |channel: Channel| {
        FerrisTopic::market(channel, exchange, symbol, params.clone())
            .ok()
            .map(|topic| topic.key())
    };
    let channel = channel.or_else(|| {
        topic
            .get("channel")
            .and_then(Value::as_str)
            .and_then(Channel::parse)
    });
    if let Some(channel) = channel {
        return key_on(channel);
    }
    let mut candidates = Channel::ALL
        .into_iter()
        .filter_map(key_on)
        .filter(|key| known(key));
    match (candidates.next(), candidates.next()) {
        (Some(key), None) => Some(key),
        _ => None,
    }
}
