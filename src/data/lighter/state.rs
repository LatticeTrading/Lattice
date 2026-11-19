//! What an L1 address's accounts hold ([`Request::Accounts`](super::Request)),
//! as Lighter writes it and as the portfolio reads it ([`MarginAccount`],
//! [`Balances`]).
//!
//! The reply lists every account of the address with its standing, its
//! positions (one per market it ever traded, most with no size) and its
//! assets. The address's own are its main account and its sub-accounts; a
//! pool it runs is listed with them and left out ([`AccountStates`]): what
//! it holds is its depositors'.
//!
//! What is read how, of one account:
//!
//! - its equity is its `total_asset_value` (its collateral with its open
//!   positions' profit and loss), its cross equity its `cross_asset_value`,
//!   and its maintenance margin the cross positions'
//!   (`cross_maintenance_margin_requirement`);
//! - the margin it holds is the cross positions' initial requirement with
//!   what is set aside for each isolated one (`allocated_margin`);
//! - what it can take out is its `available_balance`;
//! - a position's size is `position` and its direction `sign`; an entry
//!   with no size is no position. Its leverage is its market's initial
//!   margin fraction (a percentage) the other way round, and its margin
//!   that fraction of its worth (what is set aside, for an isolated one);
//! - Lighter reports no mark price with a position: it is the position's
//!   worth over its size. A liquidation price of zero is none;
//! - the funding of the account is its open positions'
//!   `total_funding_paid_out` together: what was paid out to it, below
//!   zero what it paid;
//! - an asset it holds is its `balance` (what backs its perpetuals is its
//!   collateral, counted in its equity), with what orders hold of it
//!   (`locked_balance`). Lighter does not say what a holding cost.
//!
//! An address's accounts are one report, added up as the portfolio adds up
//! accounts ([`MarginAccount::combined`], [`Balances::combined`]).
//!
//! Figures come as decimal text, some of them empty: one that does not
//! read is not reported.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::data::{
    net::Figure,
    portfolio::{Balance, Balances, Direction, Leverage, MarginAccount, MarginMode, Position},
};

/// A figure, or `None` for anything that is not one (empty text, `null`).
pub(super) fn figure<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(Figure::of(&value).map(|Figure(number)| number))
}

/// A list, or none for `null`.
pub(super) fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// The reply to [`Request::Accounts`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct AccountsReply {
    #[serde(default, deserialize_with = "list")]
    accounts: Vec<AccountState>,
}

/// One account of an address.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AccountState {
    index: u64,
    /// What kind it is: `0` the address's main account, `1` a sub-account,
    /// above that a pool it runs.
    account_type: u8,
    #[serde(deserialize_with = "figure")]
    available_balance: Option<f64>,
    #[serde(deserialize_with = "figure")]
    total_asset_value: Option<f64>,
    #[serde(deserialize_with = "figure")]
    cross_asset_value: Option<f64>,
    #[serde(deserialize_with = "figure")]
    cross_initial_margin_requirement: Option<f64>,
    #[serde(deserialize_with = "figure")]
    cross_maintenance_margin_requirement: Option<f64>,
    #[serde(deserialize_with = "list")]
    positions: Vec<PositionState>,
    #[serde(deserialize_with = "list")]
    assets: Vec<AssetState>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PositionState {
    symbol: String,
    /// Below zero short, else long.
    sign: i64,
    #[serde(deserialize_with = "figure")]
    position: Option<f64>,
    #[serde(deserialize_with = "figure")]
    avg_entry_price: Option<f64>,
    #[serde(deserialize_with = "figure")]
    position_value: Option<f64>,
    #[serde(deserialize_with = "figure")]
    unrealized_pnl: Option<f64>,
    #[serde(deserialize_with = "figure")]
    liquidation_price: Option<f64>,
    /// What funding has paid out to the position; below zero, what it paid.
    #[serde(deserialize_with = "figure")]
    total_funding_paid_out: Option<f64>,
    /// The initial margin its market asks, as a percentage (`2.00` is 50x).
    #[serde(deserialize_with = "figure")]
    initial_margin_fraction: Option<f64>,
    /// `1` for isolated, else cross.
    margin_mode: i64,
    /// What is set aside for it, when it is isolated.
    #[serde(deserialize_with = "figure")]
    allocated_margin: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AssetState {
    symbol: String,
    #[serde(deserialize_with = "figure")]
    balance: Option<f64>,
    #[serde(deserialize_with = "figure")]
    locked_balance: Option<f64>,
}

impl PositionState {
    fn isolated(&self) -> bool {
        self.margin_mode == 1
    }

    /// The position it is; none for an entry with no size.
    fn position(&self) -> Option<Position> {
        let size = self.position.map(f64::abs).filter(|size| *size > 0.0)?;
        let value = self.position_value.map(f64::abs);
        let fraction = self
            .initial_margin_fraction
            .filter(|percent| *percent > 0.0)
            .map(|percent| percent / 100.0);
        let mode = if self.isolated() {
            MarginMode::Isolated
        } else {
            MarginMode::Cross
        };
        // The margin it was opened with: its market's fraction of what it
        // was worth then.
        let opened_with = self
            .avg_entry_price
            .zip(fraction)
            .map(|(entry, fraction)| size * entry * fraction)
            .filter(|margin| *margin > 0.0);
        Some(Position {
            instrument: self.symbol.clone(),
            direction: if self.sign < 0 {
                Direction::Short
            } else {
                Direction::Long
            },
            size,
            entry_price: self.avg_entry_price,
            mark_price: value.map(|value| value / size),
            value,
            unrealized_pnl: self.unrealized_pnl,
            return_on_equity: self
                .unrealized_pnl
                .zip(opened_with)
                .map(|(pnl, margin)| pnl / margin),
            liquidation_price: self.liquidation_price.filter(|price| *price > 0.0),
            margin_used: if self.isolated() {
                self.allocated_margin
            } else {
                value
                    .zip(fraction)
                    .map(|(value, fraction)| value * fraction)
            },
            leverage: fraction.map(|fraction| Leverage {
                // To the hundredth: a third of the worth is 3x, not 3.0003x.
                value: (100.0 / fraction).round() / 100.0,
                mode,
            }),
        })
    }
}

impl AccountState {
    fn margin(&self) -> MarginAccount {
        let open: Vec<(&PositionState, Position)> = self
            .positions
            .iter()
            .filter_map(|state| Some((state, state.position()?)))
            .collect();
        let set_aside: f64 = open
            .iter()
            .filter(|(state, _)| state.isolated())
            .filter_map(|(state, _)| state.allocated_margin)
            .sum();
        let funding = open
            .iter()
            .map(|(state, _)| state.total_funding_paid_out.unwrap_or(0.0))
            .sum();
        let positions: Vec<Position> = open.into_iter().map(|(_, position)| position).collect();
        MarginAccount {
            time_ms: None,
            equity: self.total_asset_value,
            unrealized_pnl: Some(
                positions
                    .iter()
                    .filter_map(|position| position.unrealized_pnl)
                    .sum(),
            ),
            position_value: Some(positions.iter().filter_map(|position| position.value).sum()),
            margin_used: self
                .cross_initial_margin_requirement
                .map(|cross| cross + set_aside),
            maintenance_margin: self.cross_maintenance_margin_requirement,
            withdrawable: self.available_balance,
            cross_equity: self.cross_asset_value,
            funding: Some(funding),
            nearest_margin_ratio: None,
            positions,
        }
    }

    fn balances(&self) -> Balances {
        Balances(
            self.assets
                .iter()
                .filter_map(|asset| {
                    let total = asset.balance.filter(|total| *total != 0.0)?;
                    Some(Balance {
                        asset: asset.symbol.clone(),
                        total,
                        held: asset.locked_balance.unwrap_or(0.0),
                        entry_value: None,
                    })
                })
                .collect(),
        )
    }
}

/// The accounts that are an address's own trading: its main one and its
/// sub-accounts, in Lighter's order. None for an address Lighter knows no
/// account of.
#[derive(Debug, Default)]
pub(super) struct AccountStates(Vec<AccountState>);

impl From<AccountsReply> for AccountStates {
    fn from(reply: AccountsReply) -> Self {
        Self(
            reply
                .accounts
                .into_iter()
                .filter(|account| account.account_type <= 1)
                .collect(),
        )
    }
}

impl AccountStates {
    /// The accounts' indexes, which the requests about one of them name.
    pub fn indexes(&self) -> Vec<u64> {
        self.0.iter().map(|account| account.index).collect()
    }

    /// The address's margin trading: its accounts' together.
    pub fn margin(&self) -> MarginAccount {
        let parts: Vec<MarginAccount> = self.0.iter().map(AccountState::margin).collect();
        match parts.is_empty() {
            // An address with no account has nothing open, and says so.
            true => MarginAccount {
                unrealized_pnl: Some(0.0),
                position_value: Some(0.0),
                funding: Some(0.0),
                ..MarginAccount::default()
            },
            false => MarginAccount::combined(&parts),
        }
    }

    /// The assets the address holds: its accounts' together.
    pub fn balances(&self) -> Balances {
        let parts: Vec<Balances> = self.0.iter().map(AccountState::balances).collect();
        Balances::combined(&parts)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read(reply: Value) -> AccountStates {
        serde_json::from_value::<AccountsReply>(reply)
            .expect("the documented reply reads")
            .into()
    }

    /// An address with a main account, a sub-account and a pool, as
    /// Lighter lists them (the members read, and some it has besides).
    fn reply() -> Value {
        json!({
            "code": 200,
            "total": 3,
            "accounts": [
                {
                    "code": 0, "account_type": 0, "index": 2180, "account_index": 2180,
                    "l1_address": "0xe117d3d94b30F7e7aA4f29704d0d2f43C009b9A8",
                    "available_balance": "392839.851354", "collateral": "767738.799185",
                    "total_asset_value": "731199.9663539998",
                    "cross_asset_value": "731199.9663539998",
                    "cross_initial_margin_requirement": "338360.115000",
                    "cross_maintenance_margin_requirement": "185401.075500",
                    "positions": [
                        { "market_id": 0, "symbol": "ETH", "initial_margin_fraction": "2.00",
                          "sign": -1, "position": "1200.0000", "avg_entry_price": "1764.79",
                          "position_value": "2125620.000000", "unrealized_pnl": "-7870.609606",
                          "liquidation_price": "2220.7891393725295",
                          "total_funding_paid_out": "99.904560", "margin_mode": 0,
                          "allocated_margin": "0.000000" },
                        { "market_id": 1, "symbol": "BTC", "initial_margin_fraction": "2.00",
                          "sign": 1, "position": "105.00000", "avg_entry_price": "62938.4",
                          "position_value": "6581473.500000", "unrealized_pnl": "-27053.669592",
                          "liquidation_price": "57419.480693522266",
                          "total_funding_paid_out": "-516.319278", "margin_mode": 0,
                          "allocated_margin": "0.000000" },
                        { "market_id": 14, "symbol": "POL", "initial_margin_fraction": "33.33",
                          "sign": 1, "position": "0", "avg_entry_price": "0.000000",
                          "position_value": "-0.000000", "unrealized_pnl": "0.000000",
                          "liquidation_price": "0", "margin_mode": 0,
                          "allocated_margin": "0.000000" },
                        { "market_id": 93, "symbol": "XAG", "initial_margin_fraction": "33.33",
                          "sign": 1, "position": "10.00", "avg_entry_price": "30.0000",
                          "position_value": "330.000000", "unrealized_pnl": "30.000000",
                          "liquidation_price": "0", "margin_mode": 1,
                          "allocated_margin": "110.000000" },
                    ],
                    "assets": [
                        { "symbol": "LIT", "asset_id": 2, "balance": "20000.73330000",
                          "locked_balance": "150.00000000", "margin_mode": "disabled",
                          "margin_balance": "0.00000000" },
                        { "symbol": "USDC", "asset_id": 3, "balance": "0.000000",
                          "locked_balance": "0.000000", "margin_mode": "enabled",
                          "margin_balance": "767738.79918549241" },
                    ],
                    "shares": null,
                },
                {
                    "code": 0, "account_type": 1, "index": 281_474_976_689_919_u64,
                    "available_balance": "", "collateral": "0.008838",
                    "total_asset_value": "0.008838", "cross_asset_value": "0.008838",
                    "cross_initial_margin_requirement": "0.000000",
                    "cross_maintenance_margin_requirement": "0.000000",
                    "positions": null,
                    "assets": [
                        { "symbol": "LIT", "balance": "0.26670000", "locked_balance": "0" },
                    ],
                },
                {
                    "code": 0, "account_type": 3, "index": 281_474_976_710_654_u64,
                    "total_asset_value": "79030542.82430604",
                    "positions": [
                        { "symbol": "BTC", "sign": 1, "position": "9.0",
                          "position_value": "560000.0" },
                    ],
                },
            ],
        })
    }

    #[test]
    fn an_account_reads_as_its_standing_its_open_positions_and_its_assets() {
        let states = read(reply());
        // The pool it runs is not its own to count.
        assert_eq!(states.indexes(), [2180, 281_474_976_689_919]);

        let main = states.0[0].margin();
        assert_eq!(main.equity, Some(731_199.966_353_999_8));
        assert_eq!(main.cross_equity, main.equity);
        assert_eq!(main.maintenance_margin, Some(185_401.075_5));
        assert_eq!(main.withdrawable, Some(392_839.851_354));
        // The cross requirement with what the isolated position holds.
        assert_eq!(main.margin_used, Some(338_360.115 + 110.0));
        assert!((main.unrealized_pnl.unwrap() - -34_894.279_198).abs() < 1e-6);
        assert_eq!(main.position_value, Some(2_125_620.0 + 6_581_473.5 + 330.0));
        // What funding paid out to it, less what it paid.
        assert!((main.funding.unwrap() - -416.414_718).abs() < 1e-9);
        assert!(
            (main.margin_ratio().unwrap() - 185_401.075_5 / 731_199.966_353_999_8).abs() < 1e-12
        );

        // The entry with no size is no position.
        let held: Vec<&str> = main
            .positions
            .iter()
            .map(|position| position.instrument.as_str())
            .collect();
        assert_eq!(held, ["ETH", "BTC", "XAG"]);
        let eth = &main.positions[0];
        assert_eq!((eth.direction, eth.size), (Direction::Short, 1_200.0));
        assert_eq!(eth.entry_price, Some(1_764.79));
        assert_eq!(eth.mark_price, Some(1_771.35));
        assert_eq!(eth.liquidation_price, Some(2_220.789_139_372_529_5));
        // 2% of its worth, at 50x; its return on the margin it opened with.
        assert_eq!(eth.margin_used, Some(42_512.4));
        assert_eq!(
            eth.leverage,
            Some(Leverage {
                value: 50.0,
                mode: MarginMode::Cross
            })
        );
        let opened_with = 1_200.0 * 1_764.79 * 0.02;
        assert!((eth.return_on_equity.unwrap() - -7_870.609_606 / opened_with).abs() < 1e-12);
        // The isolated one holds what is set aside for it, and nothing
        // liquidates it at a price of zero.
        let silver = &main.positions[2];
        assert_eq!(silver.direction, Direction::Long);
        assert_eq!(silver.margin_used, Some(110.0));
        assert_eq!(silver.liquidation_price, None);
        assert_eq!(
            silver.leverage,
            Some(Leverage {
                value: 3.0,
                mode: MarginMode::Isolated
            })
        );

        // Its assets: what it holds, not what backs its perpetuals.
        assert_eq!(
            states.0[0].balances(),
            Balances(vec![Balance {
                asset: "LIT".to_owned(),
                total: 20_000.733_3,
                held: 150.0,
                entry_value: None,
            }])
        );
    }

    #[test]
    fn an_address_is_its_accounts_added_up() {
        let states = read(reply());
        let margin = states.margin();
        assert!((margin.equity.unwrap() - 731_199.975_191_999_8).abs() < 1e-6);
        assert_eq!(margin.positions.len(), 3);
        assert_eq!(margin.maintenance_margin, Some(185_401.075_5));
        // The sub-account's is empty text: not reported, and so neither is
        // the sum.
        assert_eq!(margin.withdrawable, None);
        // The main account's ratio: the sub-account has nothing open.
        assert!(
            (margin.margin_ratio().unwrap() - 185_401.075_5 / 731_199.966_353_999_8).abs() < 1e-12
        );
        let Balances(balances) = states.balances();
        assert_eq!(balances.len(), 1);
        assert_eq!(
            (balances[0].asset.as_str(), balances[0].total),
            ("LIT", 20_001.0)
        );

        // An address with no account: nothing open, and nothing held.
        let none = read(json!({ "code": 200, "accounts": null }));
        assert!(none.indexes().is_empty());
        assert_eq!(none.balances(), Balances::default());
        let margin = none.margin();
        assert!(margin.positions.is_empty());
        assert_eq!((margin.equity, margin.funding), (None, Some(0.0)));
    }
}
