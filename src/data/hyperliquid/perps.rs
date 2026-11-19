//! A user's perpetuals account ([`Info::ClearinghouseState`](super::Info)):
//! its margin summary and its open positions, as Hyperliquid writes them
//! and as the portfolio reads them ([`MarginAccount`]).
//!
//! What is read how:
//!
//! - the account's equity, the positions' worth and the margin they hold
//!   are the `marginSummary`'s (cross and isolated together); the
//!   maintenance margin is the cross positions' (`crossMaintenanceMarginUsed`,
//!   the only one reported), and so is the cross equity it is measured
//!   against (`crossMarginSummary`'s account value);
//! - the funding of the account is its open positions' `cumFunding.sinceOpen`
//!   together, the other way round: Hyperliquid counts what was paid, the
//!   portfolio what was received;
//! - a position's size is `szi`'s magnitude and its direction `szi`'s
//!   sign; an entry with no size is no position;
//! - Hyperliquid reports no mark price with a position: it is the
//!   position's worth over its size, which is how that worth was made;
//! - the unrealized profit and loss of the account is its positions'
//!   together.
//!
//! Under Hyperliquid's unified account the collateral is the user's token
//! balances ([`super::spot`]); this state is the perpetuals' side of the
//! account either way.
//!
//! Members Hyperliquid may add are ignored; one it leaves out reads as not
//! reported.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Direction, Leverage, MarginAccount, MarginMode, Position},
};

/// The reply to [`Info::ClearinghouseState`](super::Info).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClearinghouseState {
    #[serde(default)]
    asset_positions: Vec<AssetPosition>,
    cross_maintenance_margin_used: Option<Figure>,
    /// The cross positions' margin alone; left out by an account that
    /// never traded.
    cross_margin_summary: Option<MarginSummary>,
    #[serde(default)]
    margin_summary: MarginSummary,
    /// When the state was made, in Unix milliseconds.
    time: Option<f64>,
    withdrawable: Option<Figure>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarginSummary {
    account_value: Option<Figure>,
    total_margin_used: Option<Figure>,
    /// The positions' notional worth.
    total_ntl_pos: Option<Figure>,
}

#[derive(Debug, Deserialize)]
struct AssetPosition {
    position: PositionState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionState {
    coin: String,
    /// The signed size: above zero long, below zero short.
    szi: Figure,
    entry_px: Option<Figure>,
    position_value: Option<Figure>,
    unrealized_pnl: Option<Figure>,
    return_on_equity: Option<Figure>,
    /// `null` for a position nothing would liquidate.
    liquidation_px: Option<Figure>,
    margin_used: Option<Figure>,
    leverage: Option<LeverageState>,
    cum_funding: Option<CumFunding>,
}

/// The funding a position has paid, counted from three starts. Above zero
/// is paid, below zero received.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CumFunding {
    since_open: Option<Figure>,
}

#[derive(Debug, Deserialize)]
struct LeverageState {
    /// `cross` or `isolated`.
    #[serde(rename = "type")]
    mode: String,
    value: Figure,
}

/// A figure's number, when there is one.
fn number(figure: Option<Figure>) -> Option<f64> {
    figure.map(|Figure(number)| number)
}

impl PositionState {
    /// The position it is; none for an entry with no size.
    fn position(self) -> Option<Position> {
        let Figure(signed) = self.szi;
        if signed == 0.0 {
            return None;
        }
        let size = signed.abs();
        let value = number(self.position_value);
        Some(Position {
            instrument: self.coin,
            direction: if signed > 0.0 {
                Direction::Long
            } else {
                Direction::Short
            },
            size,
            entry_price: number(self.entry_px),
            mark_price: value.map(|value| value / size),
            value,
            unrealized_pnl: number(self.unrealized_pnl),
            return_on_equity: number(self.return_on_equity),
            liquidation_price: number(self.liquidation_px),
            margin_used: number(self.margin_used),
            leverage: self.leverage.map(|leverage| Leverage {
                value: leverage.value.0,
                mode: if leverage.mode == "isolated" {
                    MarginMode::Isolated
                } else {
                    MarginMode::Cross
                },
            }),
        })
    }
}

impl From<ClearinghouseState> for MarginAccount {
    fn from(state: ClearinghouseState) -> Self {
        // Funding is Hyperliquid's paid, which the account's is the other
        // way round: received.
        let mut funding = Some(0.0);
        let mut positions: Vec<Position> = Vec::with_capacity(state.asset_positions.len());
        for AssetPosition { position } in state.asset_positions {
            let paid = position
                .cum_funding
                .as_ref()
                .map(|funding| number(funding.since_open));
            let Some(open) = position.position() else {
                continue;
            };
            funding = match (funding, paid.flatten()) {
                (Some(sum), Some(paid)) => Some(sum - paid),
                _ => None,
            };
            positions.push(open);
        }
        let unrealized = positions
            .iter()
            .filter_map(|position| position.unrealized_pnl)
            .sum();
        Self {
            time_ms: state.time,
            equity: number(state.margin_summary.account_value),
            unrealized_pnl: Some(unrealized),
            position_value: number(state.margin_summary.total_ntl_pos),
            margin_used: number(state.margin_summary.total_margin_used),
            maintenance_margin: number(state.cross_maintenance_margin_used),
            withdrawable: number(state.withdrawable),
            cross_equity: state
                .cross_margin_summary
                .and_then(|cross| number(cross.account_value)),
            funding,
            nearest_margin_ratio: None,
            positions,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The documented reply, with a short and an empty entry added.
    fn reply() -> ClearinghouseState {
        serde_json::from_value(json!({
            "assetPositions": [
                {
                    "position": {
                        "coin": "ETH",
                        "cumFunding": {
                            "allTime": "514.085417", "sinceChange": "0.0", "sinceOpen": "0.0"
                        },
                        "entryPx": "2986.3",
                        "leverage": { "rawUsd": "-95.059824", "type": "isolated", "value": 20 },
                        "liquidationPx": "2866.26936529",
                        "marginUsed": "4.967826",
                        "maxLeverage": 50,
                        "positionValue": "100.02765",
                        "returnOnEquity": "-0.0026789",
                        "szi": "0.0335",
                        "unrealizedPnl": "-0.0134"
                    },
                    "type": "oneWay"
                },
                {
                    "position": {
                        "coin": "BTC",
                        "cumFunding": {
                            "allTime": "-40.0", "sinceChange": "0.0", "sinceOpen": "-12.5"
                        },
                        "entryPx": "60000.0",
                        "leverage": { "type": "cross", "value": 5 },
                        "liquidationPx": null,
                        "marginUsed": "1220.0",
                        "positionValue": "6100.0",
                        "returnOnEquity": "-0.0833",
                        "szi": "-0.1",
                        "unrealizedPnl": "-100.0"
                    },
                    "type": "oneWay"
                },
                { "position": { "coin": "SOL", "szi": "0.0" }, "type": "oneWay" }
            ],
            "crossMaintenanceMarginUsed": "61.0",
            "crossMarginSummary": {
                "accountValue": "13104.514502", "totalMarginUsed": "0.0",
                "totalNtlPos": "0.0", "totalRawUsd": "13104.514502"
            },
            "marginSummary": {
                "accountValue": "13109.482328", "totalMarginUsed": "1224.967826",
                "totalNtlPos": "6200.02765", "totalRawUsd": "13009.454678"
            },
            "time": 1_708_622_398_623_u64,
            "withdrawable": "13104.514502"
        }))
        .expect("the documented reply reads")
    }

    #[test]
    fn a_state_reads_as_the_account_and_its_positions() {
        let account = MarginAccount::from(reply());
        assert_eq!(account.time_ms, Some(1_708_622_398_623.0));
        assert_eq!(account.equity, Some(13_109.482_328));
        assert_eq!(account.position_value, Some(6_200.027_65));
        assert_eq!(account.margin_used, Some(1_224.967_826));
        assert_eq!(account.maintenance_margin, Some(61.0));
        assert_eq!(account.withdrawable, Some(13_104.514_502));
        assert!((account.unrealized_pnl.unwrap() + 100.0134).abs() < 1e-9);
        let leverage = account.leverage().unwrap();
        assert!((leverage - 6_200.027_65 / 13_109.482_328).abs() < 1e-12);
        // The cross margin alone, which the maintenance margin is measured
        // against.
        assert_eq!(account.cross_equity, Some(13_104.514_502));
        assert!((account.margin_ratio().unwrap() - 61.0 / 13_104.514_502).abs() < 1e-12);
        assert!((account.liquidation_buffer().unwrap() - 13_043.514_502).abs() < 1e-9);
        // Hyperliquid's funding paid is the account's received, the other
        // way round: the short has received 12.5, the long paid nothing.
        assert_eq!(account.funding, Some(12.5));

        // The entry with no size is no position.
        let [eth, btc] = account.positions.as_slice() else {
            panic!("two positions: {:?}", account.positions);
        };
        assert_eq!(eth.instrument, "ETH");
        assert_eq!((eth.direction, eth.size), (Direction::Long, 0.0335));
        assert_eq!(eth.entry_price, Some(2986.3));
        assert_eq!(eth.value, Some(100.027_65));
        assert!((eth.mark_price.unwrap() - 100.027_65 / 0.0335).abs() < 1e-9);
        assert_eq!(eth.liquidation_price, Some(2_866.269_365_29));
        assert_eq!(eth.return_on_equity, Some(-0.002_678_9));
        assert_eq!(
            eth.leverage,
            Some(Leverage {
                value: 20.0,
                mode: MarginMode::Isolated
            })
        );

        // A short's size is its magnitude; `null` is no liquidation price.
        assert_eq!((btc.direction, btc.size), (Direction::Short, 0.1));
        assert_eq!(btc.liquidation_price, None);
        assert_eq!(btc.mark_price, Some(61_000.0));
        assert_eq!(btc.leverage.unwrap().mode, MarginMode::Cross);
    }

    #[test]
    fn an_account_that_never_traded_reads_as_empty() {
        let state: ClearinghouseState = serde_json::from_value(json!({
            "assetPositions": [],
            "marginSummary": {
                "accountValue": "0.0", "totalMarginUsed": "0.0",
                "totalNtlPos": "0.0", "totalRawUsd": "0.0"
            },
            "crossMaintenanceMarginUsed": "0.0",
            "withdrawable": "0.0",
            "time": 1_708_622_398_623_u64
        }))
        .unwrap();
        let account = MarginAccount::from(state);
        assert!(account.positions.is_empty());
        assert_eq!(account.equity, Some(0.0));
        assert_eq!(account.unrealized_pnl, Some(0.0));
        assert_eq!(account.leverage(), None, "nothing to measure against");
        assert_eq!((account.cross_equity, account.margin_ratio()), (None, None));
        assert_eq!(account.funding, Some(0.0), "nothing open, nothing paid");

        // A reply that is no state at all does not read.
        assert!(serde_json::from_value::<ClearinghouseState>(json!(null)).is_err());
        let damaged = json!({ "assetPositions": [{ "position": { "coin": "ETH", "szi": "x" } }] });
        assert!(serde_json::from_value::<ClearinghouseState>(damaged).is_err());
    }
}
