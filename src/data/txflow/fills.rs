//! A user's latest trades ([`Info::UserFills`](super::Info)), as TxFlow
//! writes them and as the portfolio reads them ([`Fills`]).
//!
//! - The reply is a page, newest first (`{ "data": [...], "hasMore" }`);
//!   one request asks for the latest [`FILLS_LIMIT`](super::info::FILLS_LIMIT),
//!   the most TxFlow returns at once. Each row is one fill of one order
//!   against one other: TxFlow does not add an order's fills together.
//! - `side` is `B` (bid: a buy) or `A` (ask: a sell); whether it took
//!   liquidity is `tradeSide` (`Taker`, `Maker`), not `crossed`, which
//!   TxFlow leaves `null`.
//! - `dir` is only the side (`Buy`, `Sell`, `Liquidation: Sell`), so what
//!   the trade did is read from the position it started from
//!   (`startPosition`): `Open Long`, `Close Short`, `Long > Short` for one
//!   that turned it round, as Hyperliquid says it; a liquidation keeps
//!   TxFlow's `Liquidation` in front.
//! - A trade that only opens or adds to a position closed nothing, which
//!   reads as no closed profit and loss at all.
//! - A fill with no price, size or time is left out.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Fill, Fills, TradeSide},
};

/// The reply to [`Info::UserFills`](super::Info): one page.
#[derive(Debug, Deserialize)]
pub(super) struct UserFills {
    #[serde(default)]
    data: Vec<UserFill>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserFill {
    coin: String,
    px: Option<Figure>,
    sz: Option<Figure>,
    side: String,
    time: Option<f64>,
    dir: Option<String>,
    start_position: Option<Figure>,
    closed_pnl: Option<Figure>,
    fee: Option<Figure>,
    fee_token: Option<String>,
    trade_side: Option<String>,
    tid: Option<u64>,
    hash: Option<String>,
}

/// What a trade of `size` on `side` did to a position of `start` (signed:
/// above zero long), in Hyperliquid's words.
fn action(side: TradeSide, start: f64, size: f64) -> &'static str {
    let end = match side {
        TradeSide::Buy => start + size,
        TradeSide::Sell => start - size,
    };
    match (start, side) {
        (start, TradeSide::Buy) if start >= 0.0 => "Open Long",
        (start, TradeSide::Sell) if start <= 0.0 => "Open Short",
        (_, TradeSide::Buy) if end > 0.0 => "Short > Long",
        (_, TradeSide::Sell) if end < 0.0 => "Long > Short",
        (_, TradeSide::Buy) => "Close Short",
        (_, TradeSide::Sell) => "Close Long",
    }
}

impl UserFill {
    fn fill(self) -> Option<Fill> {
        let (Figure(price), Figure(size), time_ms) = (self.px?, self.sz?, self.time?);
        let side = match self.side.as_str() {
            "B" => TradeSide::Buy,
            "A" => TradeSide::Sell,
            _ => return None,
        };
        let start = self.start_position.map(|Figure(start)| start);
        let did = start.map(|start| action(side, start, size));
        let liquidation = self
            .dir
            .as_deref()
            .is_some_and(|dir| dir.starts_with("Liquidation"));
        let action = match did {
            Some(did) if liquidation => Some(format!("Liquidation: {did}")),
            Some(did) => Some(did.to_owned()),
            None => self.dir,
        };
        let opens = did.is_some_and(|did| did.starts_with("Open"));
        Some(Fill {
            id: self.tid.map_or_else(
                || format!("{}@{time_ms}", self.hash.unwrap_or_default()),
                |tid| tid.to_string(),
            ),
            time_ms,
            instrument: self.coin,
            side,
            action,
            price,
            size,
            fee: self.fee.map(|Figure(fee)| fee),
            fee_asset: self.fee_token.filter(|token| !token.is_empty()),
            closed_pnl: self.closed_pnl.map(|Figure(pnl)| pnl).filter(|_| !opens),
            taker: self.trade_side.as_deref().and_then(|role| match role {
                "Taker" => Some(true),
                "Maker" => Some(false),
                _ => None,
            }),
        })
    }
}

impl From<UserFills> for Fills {
    fn from(UserFills { data }: UserFills) -> Self {
        let mut fills: Vec<Fill> = data.into_iter().filter_map(UserFill::fill).collect();
        fills.sort_by(|a, b| b.time_ms.total_cmp(&a.time_ms));
        Self(fills)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fills_read_newest_first_with_what_they_did_and_closed() {
        // As TxFlow wrote them (checked live 2026-11-19), with a liquidation
        // and what is no trade added.
        let reply: UserFills = serde_json::from_value(json!({
            "data": [
                { "coin": "NIL", "symbol": "NIL-USDC", "px": "0.05069", "sz": "40", "side": "B",
                  "time": 1_795_087_836_840_u64, "startPosition": "-62910", "dir": "Buy",
                  "reduceOnly": false, "closedPnl": "0.023350923070339480297760244",
                  "hash": "0x71b2", "oid": 397_980_548_957_u64, "crossed": null, "fee": "0",
                  "tid": 384_213_274_000_004_u64, "feeToken": "USDC", "twapId": null,
                  "tradeSide": "Maker", "marginMode": "Cross", "leverage": 10,
                  "instrumentId": 201, "instrumentType": "perps", "marketType": "perps" },
                { "coin": "BTC", "px": "65010.5", "sz": "0.01", "side": "B",
                  "time": 1_795_087_000_000_u64, "startPosition": "0", "dir": "Buy",
                  "closedPnl": "0", "fee": "0.02", "tid": 987_654, "feeToken": "USDC",
                  "tradeSide": "Taker" },
                { "coin": "ETH", "px": "3000", "sz": "2", "side": "A",
                  "time": 1_795_086_000_000_u64, "startPosition": "1.5",
                  "dir": "Liquidation: Sell", "closedPnl": "-120.5", "fee": "1.2",
                  "tid": 5, "feeToken": "USDC", "tradeSide": "Taker" },
                { "coin": "SOL", "px": "150", "sz": "1", "side": "X", "time": 1 },
                { "coin": "SOL", "px": "150", "side": "B", "time": 2 }
            ],
            "hasMore": true,
            "total": 2000
        }))
        .expect("TxFlow's reply reads");
        let Fills(fills) = reply.into();
        assert_eq!(fills.len(), 3, "what is no trade is left out");
        let (nil, btc, eth) = (&fills[0], &fills[1], &fills[2]);

        assert_eq!(nil.instrument, "NIL");
        assert_eq!(nil.id, "384213274000004");
        assert_eq!(
            (nil.side, nil.action.as_deref()),
            (TradeSide::Buy, Some("Close Short"))
        );
        assert!((nil.closed_pnl.unwrap() - 0.023_350_923).abs() < 1e-9);
        assert_eq!((nil.fee, nil.taker), (Some(0.0), Some(false)));
        assert_eq!(nil.fee_asset.as_deref(), Some("USDC"));

        // An opening trade closed nothing.
        assert_eq!(btc.action.as_deref(), Some("Open Long"));
        assert_eq!((btc.closed_pnl, btc.taker), (None, Some(true)));

        // A liquidation that turned the long round.
        assert_eq!(eth.action.as_deref(), Some("Liquidation: Long > Short"));
        assert_eq!(eth.closed_pnl, Some(-120.5));
    }

    #[test]
    fn what_a_trade_did_is_read_from_where_it_started() {
        use TradeSide::{Buy, Sell};
        assert_eq!(action(Buy, 0.0, 1.0), "Open Long");
        assert_eq!(action(Buy, 2.0, 1.0), "Open Long");
        assert_eq!(action(Sell, 0.0, 1.0), "Open Short");
        assert_eq!(action(Sell, -2.0, 1.0), "Open Short");
        assert_eq!(action(Sell, 2.0, 1.0), "Close Long");
        assert_eq!(action(Sell, 2.0, 2.0), "Close Long");
        assert_eq!(action(Buy, -2.0, 2.0), "Close Short");
        assert_eq!(action(Sell, 1.0, 3.0), "Long > Short");
        assert_eq!(action(Buy, -1.0, 3.0), "Short > Long");
    }

    #[test]
    fn a_fill_without_its_start_says_what_txflow_says() {
        let reply: UserFills = serde_json::from_value(json!({
            "data": [{ "coin": "BTC", "px": "1", "sz": "1", "side": "A", "time": 1,
                       "dir": "Sell", "closedPnl": "0", "hash": "0xab" }]
        }))
        .unwrap();
        let Fills(fills) = reply.into();
        assert_eq!(fills[0].action.as_deref(), Some("Sell"));
        assert_eq!(fills[0].closed_pnl, Some(0.0));
        assert_eq!(fills[0].id, "0xab@1");
        assert_eq!(fills[0].taker, None);
    }
}
