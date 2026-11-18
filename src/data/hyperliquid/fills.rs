//! A user's latest trades ([`Info::UserFills`](super::Info)), as
//! Hyperliquid writes them and as the portfolio reads them ([`Fills`]).
//!
//! - They are asked for with `aggregateByTime`, so what one order filled at
//!   one time is one fill (its price the average) rather than one per
//!   resting order it met. Hyperliquid returns the latest 2000.
//! - `side` is `B` (bid: a buy) or `A` (ask: a sell); `dir` says what the
//!   trade did (`Open Long`, `Close Short`, `Buy` for spot), and `crossed`
//!   whether it took liquidity.
//! - `closedPnl` is `0.0` on a trade that opens a position: such a trade
//!   closed nothing, which reads as no closed profit and loss at all.
//! - A fill with no price, size or time is left out.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Fill, Fills, TradeSide},
};

/// The reply to [`Info::UserFills`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct UserFills(pub(super) Vec<UserFill>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserFill {
    coin: String,
    px: Option<Figure>,
    sz: Option<Figure>,
    side: String,
    time: Option<f64>,
    dir: Option<String>,
    closed_pnl: Option<Figure>,
    fee: Option<Figure>,
    fee_token: Option<String>,
    crossed: Option<bool>,
    tid: Option<u64>,
    hash: Option<String>,
}

impl UserFill {
    fn fill(self) -> Option<Fill> {
        let (Figure(price), Figure(size), time_ms) = (self.px?, self.sz?, self.time?);
        let side = match self.side.as_str() {
            "B" => TradeSide::Buy,
            "A" => TradeSide::Sell,
            _ => return None,
        };
        let opens = self.dir.as_deref().is_some_and(|dir| dir.starts_with("Open"));
        Some(Fill {
            id: self.tid.map_or_else(
                || format!("{}@{time_ms}", self.hash.unwrap_or_default()),
                |tid| tid.to_string(),
            ),
            time_ms,
            instrument: self.coin,
            side,
            action: self.dir,
            price,
            size,
            fee: self.fee.map(|Figure(fee)| fee),
            fee_asset: self.fee_token,
            closed_pnl: self
                .closed_pnl
                .map(|Figure(pnl)| pnl)
                .filter(|_| !opens),
            taker: self.crossed,
        })
    }
}

impl From<UserFills> for Fills {
    fn from(UserFills(fills): UserFills) -> Self {
        let mut fills: Vec<Fill> = fills.into_iter().filter_map(UserFill::fill).collect();
        fills.sort_by(|a, b| b.time_ms.total_cmp(&a.time_ms));
        Self(fills)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::data::hyperliquid::Info;

    #[test]
    fn fills_read_newest_first_with_what_they_closed() {
        let reply: UserFills = serde_json::from_value(json!([
            { "coin": "ETH", "px": "2986.3", "sz": "0.0335", "side": "B",
              "time": 1_708_622_000_000_u64, "startPosition": "0.0", "dir": "Open Long",
              "closedPnl": "0.0", "hash": "0xa", "oid": 1, "crossed": true,
              "fee": "0.05", "tid": 11, "feeToken": "USDC" },
            { "coin": "LINK", "px": "9.279895754", "sz": "68.3", "side": "A",
              "time": 1_708_622_398_623_u64, "dir": "Close Long", "closedPnl": "12.359278",
              "hash": "0xb", "crossed": false, "fee": "-0.01", "tid": 12, "feeToken": "USDC" },
            { "coin": "PURR/USDC", "px": "0.5", "sz": "100", "side": "X", "time": 1 },
            { "coin": "BTC", "px": "60000", "side": "B", "time": 2 }
        ]))
        .expect("the documented reply reads");
        let Fills(fills) = reply.into();
        assert_eq!(fills.len(), 2, "what is no trade is left out");
        let (link, eth) = (&fills[0], &fills[1]);
        assert_eq!(link.instrument, "LINK", "newest first");
        assert_eq!(
            (link.side, link.action.as_deref()),
            (TradeSide::Sell, Some("Close Long"))
        );
        assert_eq!(link.closed_pnl, Some(12.359_278));
        assert_eq!((link.fee, link.taker), (Some(-0.01), Some(false)));
        assert!((link.value() - 633.816_880_998_2).abs() < 1e-6);
        assert_eq!(link.id, "12");
        // An opening trade closed nothing.
        assert_eq!((eth.side, eth.closed_pnl), (TradeSide::Buy, None));
        assert_eq!(eth.fee_asset.as_deref(), Some("USDC"));
    }

    #[test]
    fn fills_are_asked_for_by_time_and_weigh_their_count() {
        let request = Info::UserFills {
            user: "0x01".to_owned(),
            aggregate_by_time: true,
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({ "type": "userFills", "user": "0x01", "aggregateByTime": true })
        );
        assert_eq!(request.weight(), 20);
        assert_eq!(request.item_weight(2000), 100);
        assert_eq!(request.item_weight(19), 0);
        let other = Info::UserFees {
            user: "0x01".to_owned(),
        };
        assert_eq!(other.item_weight(2000), 0);
    }
}
