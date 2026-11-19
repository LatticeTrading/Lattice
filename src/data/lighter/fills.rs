//! An account's latest trades ([`Request::Trades`](super::Request)), as
//! Lighter writes them and as the portfolio reads them ([`Fills`]).
//!
//! A trade names both of its accounts, the one that bought (`bid`) and the
//! one that sold (`ask`), and which of them made it (`is_maker_ask`). Read
//! for one account:
//!
//! - it bought when it is the bid's account, and took liquidity when the
//!   other side made the trade;
//! - its fee is its side's rate (`maker_fee` or `taker_fee`, in millionths
//!   of what was traded, left out when nothing) of the trade's worth in US
//!   dollars;
//! - what it closed is its side's realized profit and loss
//!   (`bid_account_pnl`, `ask_account_pnl`), which Lighter writes only for
//!   a trade that reduced a position;
//! - Lighter does not say what the trade did to the position in words.
//!
//! A trade is told apart by its id with its account's index: an address's
//! two accounts may be the two sides of one trade.

use serde::Deserialize;

use super::state::{figure, list};
use crate::data::portfolio::{Fill, TradeSide};

/// The asset Lighter's fees are paid in.
const FEE_ASSET: &str = "USDC";
/// What a fee rate is written in: millionths of what was traded.
const RATE_UNIT: f64 = 1_000_000.0;
/// A time above this is in milliseconds, not seconds.
const MILLISECONDS_ABOVE: f64 = 100_000_000_000.0;

/// The reply to [`Request::Trades`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct TradesReply {
    #[serde(default, deserialize_with = "list")]
    trades: Vec<Trade>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Trade {
    trade_id: u64,
    market_id: i64,
    market_symbol: String,
    #[serde(deserialize_with = "figure")]
    size: Option<f64>,
    #[serde(deserialize_with = "figure")]
    price: Option<f64>,
    #[serde(deserialize_with = "figure")]
    usd_amount: Option<f64>,
    ask_account_id: u64,
    bid_account_id: u64,
    /// Whether the seller made the trade (and the buyer took it).
    is_maker_ask: bool,
    /// When, in Unix milliseconds.
    #[serde(deserialize_with = "figure")]
    timestamp: Option<f64>,
    #[serde(deserialize_with = "figure")]
    taker_fee: Option<f64>,
    #[serde(deserialize_with = "figure")]
    maker_fee: Option<f64>,
    #[serde(deserialize_with = "figure")]
    bid_account_pnl: Option<f64>,
    #[serde(deserialize_with = "figure")]
    ask_account_pnl: Option<f64>,
}

impl Trade {
    /// The trade as `account`'s; none without its time, price or size, or
    /// when it is neither of its accounts.
    fn fill(self, account: u64) -> Option<Fill> {
        let side = if self.bid_account_id == account {
            TradeSide::Buy
        } else if self.ask_account_id == account {
            TradeSide::Sell
        } else {
            return None;
        };
        let (time, price, size) = (self.timestamp?, self.price?, self.size?);
        let made = self.is_maker_ask == (side == TradeSide::Sell);
        let rate = if made { self.maker_fee } else { self.taker_fee };
        let worth = self.usd_amount.unwrap_or(price * size);
        Some(Fill {
            id: format!("{}:{account}", self.trade_id),
            time_ms: if time > MILLISECONDS_ABOVE {
                time
            } else {
                time * 1000.0
            },
            instrument: if self.market_symbol.is_empty() {
                format!("#{}", self.market_id)
            } else {
                self.market_symbol
            },
            side,
            action: None,
            price,
            size,
            fee: Some(worth * rate.unwrap_or(0.0) / RATE_UNIT),
            fee_asset: Some(FEE_ASSET.to_owned()),
            closed_pnl: match side {
                TradeSide::Buy => self.bid_account_pnl,
                TradeSide::Sell => self.ask_account_pnl,
            },
            taker: Some(!made),
        })
    }
}

impl TradesReply {
    /// The trades as `account`'s, in the reply's order (newest first).
    pub fn fills(self, account: u64) -> Vec<Fill> {
        self.trades
            .into_iter()
            .filter_map(|trade| trade.fill(account))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_trade_reads_as_the_side_of_it_the_account_was() {
        let reply = || -> TradesReply {
            serde_json::from_value(json!({
                "code": 200,
                "next_cursor": "eyJpbmRleCI6NDEzNTU5Nzk5NDd9",
                "trades": [
                    // It bought, making the trade, and closed part of a short.
                    { "trade_id": 41_355_981_730_u64, "trade_id_str": "41355981730",
                      "type": "trade", "market_id": 1, "market_symbol": "BTC",
                      "market_kind": "perps", "size": "0.00017", "price": "62680.4",
                      "usd_amount": "10.655668", "ask_account_id": 750_305,
                      "bid_account_id": 702_384, "is_maker_ask": false,
                      "timestamp": 1_795_056_230_517_u64, "maker_fee": 28,
                      "bid_account_pnl": "-0.003763" },
                    // It sold, taking the trade, at its taker rate.
                    { "trade_id": 41_355_981_729_u64, "market_id": 2048,
                      "market_symbol": "ETH/USDC", "size": "2", "price": "1771.5",
                      "usd_amount": "3543.000000", "ask_account_id": 702_384,
                      "bid_account_id": 9, "is_maker_ask": false,
                      "timestamp": 1_795_056_230_000_u64, "taker_fee": 280, "maker_fee": 28 },
                    // Somebody else's, and one with no price.
                    { "trade_id": 3, "market_symbol": "SOL", "size": "1", "price": "70",
                      "ask_account_id": 5, "bid_account_id": 6, "timestamp": 1 },
                    { "trade_id": 4, "market_symbol": "SOL", "size": "1", "price": "",
                      "ask_account_id": 5, "bid_account_id": 702_384, "timestamp": 1 },
                ],
            }))
            .expect("the documented reply reads")
        };
        let fills = reply().fills(702_384);
        assert_eq!(fills.len(), 2);
        let bought = &fills[0];
        assert_eq!(bought.id, "41355981730:702384");
        assert_eq!(bought.time_ms, 1_795_056_230_517.0);
        assert_eq!(
            (bought.instrument.as_str(), bought.side),
            ("BTC", TradeSide::Buy)
        );
        assert_eq!((bought.price, bought.size), (62_680.4, 0.000_17));
        assert_eq!(bought.taker, Some(false));
        assert!((bought.fee.unwrap() - 10.655_668 * 0.000_028).abs() < 1e-12);
        assert_eq!(bought.fee_asset.as_deref(), Some("USDC"));
        assert_eq!(bought.closed_pnl, Some(-0.003_763));
        assert_eq!(bought.action, None);

        let sold = &fills[1];
        assert_eq!(
            (sold.instrument.as_str(), sold.side),
            ("ETH/USDC", TradeSide::Sell)
        );
        assert_eq!(sold.taker, Some(true));
        assert!((sold.fee.unwrap() - 3_543.0 * 0.000_28).abs() < 1e-12);
        assert_eq!(sold.closed_pnl, None);

        // The other side of the first trade took it, and paid nothing.
        let other = reply().fills(750_305);
        assert_eq!(other.len(), 1);
        assert_eq!(
            (other[0].side, other[0].taker),
            (TradeSide::Sell, Some(true))
        );
        assert_eq!(other[0].fee, Some(0.0));
        assert_eq!(other[0].id, "41355981730:750305");

        let none: TradesReply =
            serde_json::from_value(json!({ "code": 200, "trades": null })).unwrap();
        assert!(none.fills(1).is_empty());
    }
}
