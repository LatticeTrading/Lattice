//! A user's perpetuals sub-account ([`Info::ClearinghouseState`](super::Info)
//! with `accountType: perp`): its margin summary and its open positions, as
//! TxFlow writes them and as the portfolio reads them ([`MarginAccount`]).
//!
//! The reply is Hyperliquid's shape ([`crate::data::hyperliquid`]) with a
//! few differences, so it is read the same way where it is the same:
//!
//! - the account's equity, the positions' worth and the margin they hold
//!   are the `marginSummary`'s; the maintenance margin is the cross
//!   positions' (`crossMaintenanceMarginUsed`), measured against the cross
//!   equity (`crossMarginSummary`'s account value). TxFlow's own
//!   `crossMarginRatio` is the inverse (higher is safer) and counts the
//!   liquidation fee in, so it is not read;
//! - the funding of the account is its open positions' `cumFunding.sinceOpen`
//!   together, the other way round: TxFlow counts what was paid (a short
//!   receiving funding reads below zero), the portfolio what was received;
//! - a position's size is `szi`'s magnitude and its direction `szi`'s
//!   sign; an entry with no size is no position;
//! - unlike Hyperliquid, TxFlow reports a position's mark price (`markPx`),
//!   writes its leverage's mode capitalized (`Cross`, `Isolated`) and its
//!   return on equity in percent (`85.88`), which reads as a fraction;
//! - the instrument is TxFlow's market name (`BTC-USDC`).
//!
//! Members TxFlow may add are ignored; one it leaves out reads as not
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
    mark_px: Option<Figure>,
    position_value: Option<Figure>,
    unrealized_pnl: Option<Figure>,
    /// In percent.
    return_on_equity: Option<Figure>,
    /// `null` for a position nothing would liquidate.
    liquidation_px: Option<Figure>,
    margin_used: Option<Figure>,
    leverage: Option<LeverageState>,
    cum_funding: Option<CumFunding>,
}

/// The funding a position has paid, counted from three starts. Above zero
/// is paid, below zero received; TxFlow writes an empty text for a count
/// it does not keep.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CumFunding {
    #[serde(default, deserialize_with = "lenient")]
    since_open: Option<Figure>,
}

#[derive(Debug, Deserialize)]
struct LeverageState {
    /// `Cross` or `Isolated`.
    #[serde(rename = "type")]
    mode: String,
    value: Figure,
}

/// A figure, or none for anything that is not one (`""`).
fn lenient<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Figure>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(Figure::of(&value))
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
            mark_price: number(self.mark_px).or_else(|| value.map(|value| value / size)),
            value,
            unrealized_pnl: number(self.unrealized_pnl),
            return_on_equity: number(self.return_on_equity).map(|percent| percent / 100.0),
            liquidation_price: number(self.liquidation_px),
            margin_used: number(self.margin_used),
            leverage: self.leverage.map(|leverage| Leverage {
                value: leverage.value.0,
                mode: if leverage.mode.eq_ignore_ascii_case("isolated") {
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
        // Funding is TxFlow's paid, which the account's is the other way
        // round: received.
        let mut funding = Some(0.0);
        let mut positions: Vec<Position> = Vec::with_capacity(state.asset_positions.len());
        for AssetPosition { position } in state.asset_positions {
            let paid = position
                .cum_funding
                .as_ref()
                .and_then(|funding| funding.since_open);
            let Some(open) = position.position() else {
                continue;
            };
            funding = match (funding, paid) {
                (Some(sum), Some(Figure(paid))) => Some(sum - paid),
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

    /// A reply as TxFlow made it (checked live 2026-11-19), cut to two
    /// positions, with an isolated one and an empty entry added.
    fn reply() -> ClearinghouseState {
        serde_json::from_value(json!({
            "assetPositions": [
                {
                    "position": {
                        "coin": "DYDX-USDC", "asset": "96", "entryPx": "0.0940",
                        "leverage": { "type": "Cross", "value": 5, "rawUsd": null },
                        "liquidationPx": "37.9051", "marginUsed": "2285.229428",
                        "positionValue": "11426.147141", "returnOnEquity": "85.885920",
                        "szi": "-142395", "unrealizedPnl": "1962.690340", "maxLeverage": 4,
                        "cumFunding": {
                            "allTime": "", "sinceOpen": "-1649.683217", "sinceChange": "-1649.683217"
                        },
                        "marginRatio": "56.7344", "adlQuantile": 5, "markPx": "0.0802",
                        "updateTime": 1_795_087_810_402_u64, "cumFee": "0.000000",
                        "closedPnl": "15960.323658"
                    },
                    "type": "OneWay"
                },
                {
                    "position": {
                        "coin": "COST-USDC", "asset": "211", "entryPx": "1103.16",
                        "leverage": { "type": "Isolated", "value": 10, "rawUsd": "-14000.0" },
                        "liquidationPx": null, "marginUsed": "1593.433422",
                        "positionValue": "15934.334228", "returnOnEquity": "-8.712513",
                        "szi": "14.57", "unrealizedPnl": "-138.828095",
                        "cumFunding": { "allTime": "", "sinceOpen": "46.073562", "sinceChange": "" },
                        "markPx": "1093.63"
                    },
                    "type": "OneWay"
                },
                { "position": { "coin": "BTC-USDC", "szi": "0" }, "type": "OneWay" }
            ],
            "crossMarginSummary": {
                "accountValue": "5622239.438634", "totalMarginUsed": "785813.630676",
                "totalNtlPos": "4943200.506126", "totalRawUsd": "5628742.499038"
            },
            "marginSummary": {
                "accountValue": "5622239.438634", "totalMarginUsed": "785813.630676",
                "totalNtlPos": "4943200.506126", "totalRawUsd": "5628742.499038"
            },
            "crossMaintenanceMarginUsed": "96873.082649",
            "crossMarginRatio": "56.7344",
            "withdrawable": "4836425.807958",
            "availableToTrade": "4836425.807958",
            "vaultEquity": "0.000000",
            "time": 1_795_087_837_004_u64
        }))
        .expect("TxFlow's reply reads")
    }

    #[test]
    fn a_state_reads_as_the_account_and_its_positions() {
        let account = MarginAccount::from(reply());
        assert_eq!(account.time_ms, Some(1_795_087_837_004.0));
        assert_eq!(account.equity, Some(5_622_239.438_634));
        assert_eq!(account.position_value, Some(4_943_200.506_126));
        assert_eq!(account.margin_used, Some(785_813.630_676));
        assert_eq!(account.maintenance_margin, Some(96_873.082_649));
        assert_eq!(account.withdrawable, Some(4_836_425.807_958));
        assert_eq!(account.cross_equity, Some(5_622_239.438_634));
        let ratio = account.margin_ratio().unwrap();
        assert!((ratio - 96_873.082_649 / 5_622_239.438_634).abs() < 1e-12);
        assert!((account.unrealized_pnl.unwrap() - 1_823.862_245).abs() < 1e-6);
        // The short has received 1649.68 and the long paid 46.07.
        assert!((account.funding.unwrap() - 1_603.609_655).abs() < 1e-6);

        let [dydx, cost] = account.positions.as_slice() else {
            panic!("two positions: {:?}", account.positions);
        };
        assert_eq!(dydx.instrument, "DYDX-USDC");
        assert_eq!((dydx.direction, dydx.size), (Direction::Short, 142_395.0));
        assert_eq!(dydx.mark_price, Some(0.0802), "TxFlow's own mark price");
        assert_eq!(dydx.entry_price, Some(0.094));
        assert_eq!(dydx.liquidation_price, Some(37.9051));
        assert!((dydx.return_on_equity.unwrap() - 0.858_859_2).abs() < 1e-12);
        assert_eq!(
            dydx.leverage,
            Some(Leverage {
                value: 5.0,
                mode: MarginMode::Cross
            })
        );
        assert_eq!((cost.direction, cost.size), (Direction::Long, 14.57));
        assert_eq!(cost.liquidation_price, None);
        assert_eq!(cost.leverage.unwrap().mode, MarginMode::Isolated);
    }

    #[test]
    fn an_account_that_never_traded_reads_as_empty() {
        let state: ClearinghouseState = serde_json::from_value(json!({
            "assetPositions": [],
            "crossMarginSummary": {
                "accountValue": "0.000000", "totalMarginUsed": "0.000000",
                "totalNtlPos": "0.000000", "totalRawUsd": "0.000000"
            },
            "marginSummary": {
                "accountValue": "0.000000", "totalMarginUsed": "0.000000",
                "totalNtlPos": "0.000000", "totalRawUsd": "0.000000"
            },
            "crossMaintenanceMarginUsed": "0.000000", "crossMarginRatio": "0.0000",
            "withdrawable": "0.000000", "time": 1_795_087_870_089_u64
        }))
        .unwrap();
        let account = MarginAccount::from(state);
        assert!(account.positions.is_empty());
        assert_eq!(
            (account.equity, account.unrealized_pnl),
            (Some(0.0), Some(0.0))
        );
        assert_eq!((account.leverage(), account.margin_ratio()), (None, None));
        assert_eq!(account.funding, Some(0.0));

        let damaged = json!({ "assetPositions": [{ "position": { "coin": "ETH", "szi": "x" } }] });
        assert!(serde_json::from_value::<ClearinghouseState>(damaged).is_err());
    }
}
