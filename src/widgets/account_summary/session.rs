//! The Account Summary's session ([`AccountSummarySession`]): the reports
//! of each account its settings name, and what its view shows of them
//! together (the widget's data, described in the parent module): the
//! accounts' [`Overview`] and their [`Holding`]s.
//!
//! Several accounts make one overview ([`Overview::of_accounts`]):
//!
//! - what they are worth and made adds up: their margin as the portfolio
//!   adds margin accounts up, their histories as it combines histories,
//!   and what each holds, has staked and has in vaults at its own
//!   exchange's prices;
//! - the cross margin ratio is that of the account nearest liquidation,
//!   since each is liquidated on its own;
//! - what only some of them have (staking, vaults, balances) is the sum of
//!   those that report it: an exchange that reports no staking has none to
//!   add. Their history is all of theirs or none: what the whole was worth
//!   and made is not said from some of it;
//! - a fee schedule is one account's: with several reporting theirs, the
//!   fee figures read a dash.
//!
//! Everything shown is made here from the reports, so the view only lays
//! it out and the figures are tested without one.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::settings::AccountSummarySettings;
use crate::{
    data::{
        net::Watch,
        portfolio::{
            Account, Balance, Balances, Direction, Fees, MarginAccount, MarginMode, Performance,
            Period, Position, Prices, Scope, Staking, Unavailable, Vaults,
        },
        Portfolio, SessionScope,
    },
    format,
    widgets::common::{
        accounts::{follow_all, shown, AccountSettings, Mark, Reports as Followed, Tone, NONE},
        Shared,
    },
};

/// What a holding is worth at least to show while small balances are
/// hidden, in the quote currency.
pub(super) const SMALL: f64 = 1.0;

/// One figure of the summary: a row of one of its tiles, or its headline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Figure {
    Total,
    DayChange,
    DayReturn,
    MarginRatio,
    Buffer,
    Maintenance,
    CrossEquity,
    Equity,
    MarginUsed,
    Withdrawable,
    Leverage,
    Isolated,
    IsolatedMargin,
    Long,
    Short,
    Net,
    Gross,
    Positions,
    Largest,
    Unrealized,
    Funding,
    WeekPnl,
    MonthPnl,
    DayVolume,
    FeeTier,
    FeeVolume,
    NextTier,
    PerpRates,
    SpotRates,
    Discount,
    Staked,
    StakedValue,
    Undelegated,
    Unstaking,
    VaultEquity,
    VaultCount,
}

impl Figure {
    /// Every figure, in the order [`Overview`] keeps them.
    pub const ALL: [Self; 36] = [
        Self::Total,
        Self::DayChange,
        Self::DayReturn,
        Self::MarginRatio,
        Self::Buffer,
        Self::Maintenance,
        Self::CrossEquity,
        Self::Equity,
        Self::MarginUsed,
        Self::Withdrawable,
        Self::Leverage,
        Self::Isolated,
        Self::IsolatedMargin,
        Self::Long,
        Self::Short,
        Self::Net,
        Self::Gross,
        Self::Positions,
        Self::Largest,
        Self::Unrealized,
        Self::Funding,
        Self::WeekPnl,
        Self::MonthPnl,
        Self::DayVolume,
        Self::FeeTier,
        Self::FeeVolume,
        Self::NextTier,
        Self::PerpRates,
        Self::SpotRates,
        Self::Discount,
        Self::Staked,
        Self::StakedValue,
        Self::Undelegated,
        Self::Unstaking,
        Self::VaultEquity,
        Self::VaultCount,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Total => "Total Value",
            Self::DayChange => "24H Change",
            Self::DayReturn => "24H Return",
            Self::MarginRatio => "Cross Margin Ratio",
            Self::Buffer => "Liquidation Buffer",
            Self::Maintenance => "Maintenance Margin",
            Self::CrossEquity => "Cross Equity",
            Self::Equity => "Perps Equity",
            Self::MarginUsed => "Margin Used",
            Self::Withdrawable => "Withdrawable",
            Self::Leverage => "Leverage",
            Self::Isolated => "Isolated Positions",
            Self::IsolatedMargin => "Isolated Margin",
            Self::Long => "Long",
            Self::Short => "Short",
            Self::Net => "Net Exposure",
            Self::Gross => "Position Value",
            Self::Positions => "Open Positions",
            Self::Largest => "Largest",
            Self::Unrealized => "Unrealized PnL",
            Self::Funding => "Funding Since Open",
            Self::WeekPnl => "7D PnL",
            Self::MonthPnl => "30D PnL",
            Self::DayVolume => "24H Volume",
            Self::FeeTier => "Fee Tier",
            Self::FeeVolume => "14D Volume",
            Self::NextTier => "Next Tier",
            Self::PerpRates => "Perps Rate",
            Self::SpotRates => "Spot Rate",
            Self::Discount => "Discount",
            Self::Staked => "Staked",
            Self::StakedValue => "Staked Value",
            Self::Undelegated => "Undelegated",
            Self::Unstaking => "Unstaking",
            Self::VaultEquity => "Vault Equity",
            Self::VaultCount => "Vaults",
        }
    }

    /// What its row's tooltip says, for a figure whose name leaves it out.
    pub const fn hint(self) -> Option<&'static str> {
        match self {
            Self::PerpRates | Self::SpotRates => Some("Taker / Maker"),
            _ => None,
        }
    }

    /// What its row's test id ends with.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::DayChange => "day-change",
            Self::DayReturn => "day-return",
            Self::MarginRatio => "margin-ratio",
            Self::Buffer => "liquidation-buffer",
            Self::Maintenance => "maintenance-margin",
            Self::CrossEquity => "cross-equity",
            Self::Equity => "equity",
            Self::MarginUsed => "margin-used",
            Self::Withdrawable => "withdrawable",
            Self::Leverage => "leverage",
            Self::Isolated => "isolated-positions",
            Self::IsolatedMargin => "isolated-margin",
            Self::Long => "long",
            Self::Short => "short",
            Self::Net => "net-exposure",
            Self::Gross => "position-value",
            Self::Positions => "open-positions",
            Self::Largest => "largest",
            Self::Unrealized => "unrealized-pnl",
            Self::Funding => "funding",
            Self::WeekPnl => "week-pnl",
            Self::MonthPnl => "month-pnl",
            Self::DayVolume => "day-volume",
            Self::FeeTier => "fee-tier",
            Self::FeeVolume => "fee-volume",
            Self::NextTier => "next-tier",
            Self::PerpRates => "perp-rates",
            Self::SpotRates => "spot-rates",
            Self::Discount => "discount",
            Self::Staked => "staked",
            Self::StakedValue => "staked-value",
            Self::Undelegated => "undelegated",
            Self::Unstaking => "unstaking",
            Self::VaultEquity => "vault-equity",
            Self::VaultCount => "vault-count",
        }
    }
}

/// What a figure reads, and what colors it (a gain or a loss); `None` is
/// primary text.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
    pub text: String,
    pub tone: Option<Tone>,
}

impl Shown {
    fn plain(text: String) -> Self {
        Self { text, tone: None }
    }

    /// `amount` gained or lost, signed and toned; a dash when not reported.
    fn gain(amount: Option<f64>) -> Self {
        Self {
            text: shown(amount, format::signed_usd),
            tone: amount.map(Tone::of),
        }
    }

    fn none() -> Self {
        Self::plain(NONE.to_owned())
    }
}

/// How near the cross positions are to liquidation, by the margin ratio:
/// what colors its gauge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Health {
    /// Under half the cross margin is maintenance.
    Safe,
    /// Under four fifths.
    Caution,
    /// Four fifths or more: liquidation is at all of it.
    Danger,
}

impl Health {
    pub fn of(ratio: f64) -> Self {
        if ratio < 0.5 {
            Self::Safe
        } else if ratio < 0.8 {
            Self::Caution
        } else {
            Self::Danger
        }
    }
}

/// A part of what the account is worth, as its share of the whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    /// The perpetuals account (the venue's own dex).
    Perps,
    /// The token balances, at the tokens' prices.
    Spot,
    Staked,
    Vaults,
    /// What the venue counts in the account's value and the parts above do
    /// not (Hyperliquid: positions on other perpetuals dexes).
    Other,
}

impl Part {
    /// Every part, in the bar's order.
    pub const ALL: [Self; 5] = [
        Self::Perps,
        Self::Spot,
        Self::Staked,
        Self::Vaults,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Perps => "Perps",
            Self::Spot => "Spot",
            Self::Staked => "Staked",
            Self::Vaults => "Vaults",
            Self::Other => "Other",
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Perps => "perps",
            Self::Spot => "spot",
            Self::Staked => "staked",
            Self::Vaults => "vaults",
            Self::Other => "other",
        }
    }
}

/// One part's share, as the bar and its legend show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Share {
    /// Of the whole, `0.0` to `1.0`; `0.0` hides it.
    pub fraction: f64,
    pub value: String,
    pub percent: String,
}

/// The reports of one account the summary is made of, each `None` until
/// it arrives (or when the venue does not make it).
#[derive(Clone, Copy, Default)]
pub(super) struct Reports<'a> {
    pub margin: Option<&'a MarginAccount>,
    pub balances: Option<&'a Balances>,
    pub prices: Option<&'a Prices>,
    pub performance: Option<&'a Performance>,
    pub staking: Option<&'a Staking>,
    pub vaults: Option<&'a Vaults>,
    pub fees: Option<&'a Fees>,
}

/// What the summary shows of its account: every figure, and the fractions
/// its bars are drawn to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Overview {
    /// What each figure reads, in [`Figure::ALL`]'s order.
    figures: Vec<Shown>,
    /// The parts of the account's worth, in [`Part::ALL`]'s order.
    shares: Vec<Share>,
    /// The cross margin ratio (liquidation at `1.0`).
    pub margin_ratio: Option<f64>,
    /// How much of the perpetuals' equity their margin holds.
    pub margin_usage: Option<f64>,
    /// What is long and what is short, each a fraction of both together.
    pub sides: Option<(f64, f64)>,
    /// How far the volume is from its tier's start to the next's (`1.0` at
    /// the top tier).
    pub tier_progress: Option<f64>,
}

/// `fraction` as a percentage with `places` decimals (`41.80%`).
fn percent(fraction: f64, places: usize) -> String {
    format!("{}%", format::grouped(fraction * 100.0, places))
}

/// A fee rate to four decimals of a percent, its trailing zeros dropped:
/// `0.036%`, `0.0108%`, and `0%` for none.
fn rate(fraction: f64) -> String {
    let figure = format::grouped(fraction * 100.0, 4);
    let trimmed = if figure.contains('.') {
        figure.trim_end_matches('0').trim_end_matches('.')
    } else {
        figure.as_str()
    };
    format!("{trimmed}%")
}

/// A large amount of dollars in short (`$25M`).
fn compact_usd(amount: f64) -> String {
    format!("${}", format::compact_amount(&format!("{amount:.2}")))
}

/// What `balances` are worth at `prices`; the assets that are not priced
/// are left out.
fn spot_value(balances: &Balances, prices: &Prices) -> f64 {
    balances
        .0
        .iter()
        .filter_map(|balance| Some(balance.total * prices.of(&balance.asset)?))
        .sum()
}

/// `amount` of each asset `stakes` are in, the assets' together
/// (`12 HYPE`, `12 HYPE + 5 LIT`); a dash with none.
fn staked_amounts(stakes: &[&Staking], amount: fn(&Staking) -> f64) -> String {
    let mut assets: Vec<(&str, f64)> = Vec::new();
    for staking in stakes {
        match assets.iter_mut().find(|(asset, _)| *asset == staking.asset) {
            Some((_, sum)) => *sum += amount(staking),
            None => assets.push((&staking.asset, amount(staking))),
        }
    }
    if assets.is_empty() {
        return NONE.to_owned();
    }
    let written: Vec<String> = assets
        .iter()
        .map(|(asset, sum)| format!("{} {asset}", format::quantity(*sum)))
        .collect();
    written.join(" + ")
}

impl Overview {
    /// The overview of one account's `reports`; a dash for every figure
    /// they do not make.
    pub fn of(reports: Reports<'_>) -> Self {
        Self::of_accounts(&[reports])
    }

    /// The overview of `accounts` together (each its own reports); a dash
    /// for every figure none of them makes.
    pub fn of_accounts(accounts: &[Reports<'_>]) -> Self {
        // Their margin and their histories, each as one.
        let margins: Vec<&MarginAccount> = accounts.iter().filter_map(|a| a.margin).collect();
        let combined =
            (!margins.is_empty()).then(|| MarginAccount::combined(margins.iter().copied()));
        let margin = combined.as_ref();
        let histories: Vec<&Performance> = accounts.iter().filter_map(|a| a.performance).collect();
        // The histories say what the whole is worth and made when each
        // account has one: a sum of some would read as the whole.
        let history = (!histories.is_empty() && histories.len() == accounts.len())
            .then(|| Performance::combined(histories.iter().copied()));
        let performance = history.as_ref();
        // A fee schedule is one account's.
        let fees = {
            let mut reported = accounts.iter().filter_map(|a| a.fees);
            match (reported.next(), reported.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        };
        let stakes: Vec<&Staking> = accounts.iter().filter_map(|a| a.staking).collect();
        // What the accounts that report it have of something, together.
        let added = |worth: fn(&Reports<'_>) -> Option<f64>| -> Option<f64> {
            let parts: Vec<f64> = accounts.iter().filter_map(worth).collect();
            (!parts.is_empty()).then(|| parts.iter().sum())
        };
        let window = |period: Period| performance.and_then(|p| p.window(period, Scope::Total));
        let pnl = |period: Period| {
            window(period)
                .and_then(|window| window.pnl.last())
                .map(|point| point.value)
        };
        let day = window(Period::Day);

        // What the account is worth, and of what.
        let perps = margin.and_then(|margin| margin.equity);
        // Each account's holdings at its own exchange's prices.
        let spot = added(|a| a.balances.zip(a.prices).map(|(b, p)| spot_value(b, p)));
        let staked_value = added(|a| {
            let (staking, prices) = a.staking.zip(a.prices)?;
            Some(staking.total() * prices.of(&staking.asset)?)
        });
        let vault_equity = added(|a| a.vaults.map(Vaults::equity));
        let vault_count = added(|a| a.vaults.map(|vaults| vaults.0.len() as f64));
        let parts = [perps, spot, staked_value, vault_equity];
        let counted: f64 = parts.iter().flatten().sum();
        let reported = day
            .and_then(|day| day.value.last())
            .map(|point| point.value);
        let total = reported.or_else(|| parts.iter().any(Option::is_some).then_some(counted));
        // A remainder too small to see is the reports' timing, not a part.
        let other = reported
            .map(|total| total - counted)
            .filter(|other| *other > reported.unwrap_or(0.0).abs() * 0.001 && *other >= 0.01);
        let whole = total.unwrap_or(0.0).max(counted + other.unwrap_or(0.0));
        let shares = [perps, spot, staked_value, vault_equity, other]
            .map(|part| {
                let value = part.unwrap_or(0.0).max(0.0);
                let fraction = if whole > 0.0 { value / whole } else { 0.0 };
                Share {
                    fraction,
                    value: shown(part, format::usd),
                    percent: percent(fraction, 1),
                }
            })
            .to_vec();

        // The day's change, counted from what the account was worth when
        // the day began.
        let change = pnl(Period::Day);
        let opened = day
            .and_then(|day| day.value.first())
            .map(|point| point.value);
        let day_return = change
            .zip(opened)
            .and_then(|(change, opened)| (opened > 0.0).then(|| change / opened));

        // Exposure, from the open positions.
        let sided = |side: Direction| {
            margin.map(|margin| {
                margin
                    .positions
                    .iter()
                    .filter(|position| position.direction == side)
                    .filter_map(|position| position.value)
                    .sum::<f64>()
            })
        };
        let (long, short) = (sided(Direction::Long), sided(Direction::Short));
        let gross = long.zip(short).map(|(long, short)| long + short);
        let sides = long
            .zip(short)
            .and_then(|(long, short)| (long + short > 0.0).then_some((long, short)))
            .map(|(long, short)| (long / (long + short), short / (long + short)));
        let largest = margin.and_then(|margin| {
            let gross = gross.filter(|gross| *gross > 0.0)?;
            let top = margin
                .positions
                .iter()
                .filter_map(|position| Some((position, position.value?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            Some(format!(
                "{} · {}",
                top.0.instrument,
                percent(top.1 / gross, 0)
            ))
        });

        // The isolated positions, each with margin of its own the cross
        // margin ratio leaves out.
        let isolated: Option<Vec<&Position>> = margin.map(|margin| {
            margin
                .positions
                .iter()
                .filter(|position| {
                    position
                        .leverage
                        .is_some_and(|leverage| leverage.mode == MarginMode::Isolated)
                })
                .collect()
        });
        let isolated_margin = isolated.as_ref().map(|isolated| {
            isolated
                .iter()
                .filter_map(|position| position.margin_used)
                .sum::<f64>()
        });

        let margin_ratio = margin.and_then(MarginAccount::margin_ratio);
        let margin_usage = margin.and_then(|margin| {
            let (used, equity) = (margin.margin_used?, margin.equity?);
            (equity > 0.0).then(|| (used / equity).clamp(0.0, 1.0))
        });

        // Fees.
        let tier = fees.and_then(|fees| fees.tier);
        let tier_progress = fees.and_then(|fees| {
            let (volume, floor) = (fees.volume?, fees.tier_floor?);
            Some(match fees.next_tier {
                Some(next) if next > floor => ((volume - floor) / (next - floor)).clamp(0.0, 1.0),
                Some(_) => 0.0,
                None => 1.0,
            })
        });
        let next_tier = fees.and_then(|fees| {
            let tier = fees.tier?;
            Some(fees.next_tier.map_or_else(
                || "Top tier".to_owned(),
                |next| format!("Tier {} at {}", tier + 1, compact_usd(next)),
            ))
        });
        let rates = |taker: Option<f64>, maker: Option<f64>| match taker.zip(maker) {
            Some((taker, maker)) => format!("{} / {}", rate(taker), rate(maker)),
            None => NONE.to_owned(),
        };

        // Staking, in its token.
        let staked = |amount: fn(&Staking) -> f64| staked_amounts(&stakes, amount);
        let usd = |figure: Option<f64>| Shown::plain(shown(figure, format::usd));

        let figures = Figure::ALL
            .iter()
            .map(|figure| match figure {
                Figure::Total => usd(total),
                Figure::DayChange => Shown::gain(change),
                Figure::DayReturn => Shown {
                    text: shown(day_return, format::signed_percent),
                    tone: day_return.map(|fraction| Tone::of(fraction * 100.0)),
                },
                Figure::MarginRatio => Shown::plain(shown(margin_ratio, |ratio| percent(ratio, 2))),
                Figure::Buffer => usd(margin.and_then(MarginAccount::liquidation_buffer)),
                Figure::Maintenance => usd(margin.and_then(|margin| margin.maintenance_margin)),
                Figure::CrossEquity => usd(margin.and_then(|margin| margin.cross_equity)),
                Figure::Equity => usd(perps),
                Figure::MarginUsed => usd(margin.and_then(|margin| margin.margin_used)),
                Figure::Withdrawable => usd(margin.and_then(|margin| margin.withdrawable)),
                Figure::Leverage => Shown::plain(shown(
                    margin.and_then(MarginAccount::leverage),
                    |leverage| format!("{}x", format::grouped(leverage, 2)),
                )),
                Figure::Isolated => Shown::plain(
                    isolated
                        .as_ref()
                        .map_or_else(|| NONE.to_owned(), |isolated| isolated.len().to_string()),
                ),
                Figure::IsolatedMargin => usd(isolated_margin),
                Figure::Long => usd(long),
                Figure::Short => usd(short),
                Figure::Net => Shown::plain(shown(
                    long.zip(short).map(|(long, short)| long - short),
                    format::signed_usd,
                )),
                Figure::Gross => usd(gross),
                Figure::Positions => Shown::plain(margin.map_or_else(
                    || NONE.to_owned(),
                    |margin| margin.positions.len().to_string(),
                )),
                Figure::Largest => Shown::plain(largest.clone().unwrap_or_else(|| NONE.to_owned())),
                Figure::Unrealized => Shown::gain(margin.and_then(|margin| margin.unrealized_pnl)),
                Figure::Funding => Shown::gain(margin.and_then(|margin| margin.funding)),
                Figure::WeekPnl => Shown::gain(pnl(Period::Week)),
                Figure::MonthPnl => Shown::gain(pnl(Period::Month)),
                Figure::DayVolume => usd(day.and_then(|day| day.volume)),
                Figure::FeeTier => Shown::plain(
                    tier.map_or_else(|| NONE.to_owned(), |tier| format!("Tier {tier}")),
                ),
                Figure::FeeVolume => usd(fees.and_then(|fees| fees.volume)),
                Figure::NextTier => {
                    Shown::plain(next_tier.clone().unwrap_or_else(|| NONE.to_owned()))
                }
                Figure::PerpRates => Shown::plain(
                    fees.map_or_else(|| NONE.to_owned(), |fees| rates(fees.taker, fees.maker)),
                ),
                Figure::SpotRates => Shown::plain(fees.map_or_else(
                    || NONE.to_owned(),
                    |fees| rates(fees.spot_taker, fees.spot_maker),
                )),
                Figure::Discount => {
                    Shown::plain(shown(fees.and_then(|fees| fees.discount), |discount| {
                        percent(discount, 1)
                    }))
                }
                Figure::Staked => Shown::plain(staked(|staking| staking.delegated)),
                Figure::StakedValue => usd(staked_value),
                Figure::Undelegated => Shown::plain(staked(|staking| staking.undelegated)),
                Figure::Unstaking => Shown::plain(staked(|staking| staking.pending_withdrawal)),
                Figure::VaultEquity => usd(vault_equity),
                Figure::VaultCount => {
                    Shown::plain(shown(vault_count, |count| format::grouped(count, 0)))
                }
            })
            .collect();
        Self {
            figures,
            shares,
            margin_ratio,
            margin_usage,
            sides,
            tier_progress,
        }
    }

    /// What `figure` reads.
    pub fn figure(&self, figure: Figure) -> &Shown {
        static DASH: std::sync::OnceLock<Shown> = std::sync::OnceLock::new();
        Figure::ALL
            .iter()
            .position(|kept| *kept == figure)
            .and_then(|at| self.figures.get(at))
            .unwrap_or_else(|| DASH.get_or_init(Shown::none))
    }

    /// `part`'s share of the account's worth.
    pub fn share(&self, part: Part) -> Option<&Share> {
        let at = Part::ALL.iter().position(|kept| *kept == part)?;
        self.shares.get(at)
    }
}

/// One asset an account holds, as its row shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Holding {
    /// What tells the row apart: the asset's name, after its account when
    /// the widget shows several.
    pub key: Rc<str>,
    /// Whose it is, when the widget shows several accounts.
    pub owner: Option<Rc<Mark>>,
    /// The exchange's name for the asset.
    pub asset: String,
    pub total: String,
    /// What open orders do not hold of it.
    pub available: String,
    /// What it is worth now, a dash when it is not priced.
    pub value: String,
    /// What it cost, a dash when not reported.
    pub cost: String,
    /// What it has made since: its worth less its cost, when both are known
    /// and it cost something.
    pub pnl: String,
    pub tone: Option<Tone>,
    /// Its worth, which orders the rows.
    worth: Option<f64>,
}

impl Holding {
    fn of(balance: &Balance, prices: Option<&Prices>, owner: Option<&Rc<Mark>>) -> Self {
        let worth = prices
            .and_then(|prices| prices.of(&balance.asset))
            .map(|price| balance.total * price);
        let made = worth
            .zip(balance.entry_value.filter(|cost| *cost > 0.0))
            .map(|(worth, cost)| worth - cost);
        Self {
            key: Mark::key(owner, &balance.asset),
            owner: owner.cloned(),
            asset: balance.asset.clone(),
            total: format::quantity(balance.total),
            available: format::quantity(balance.available()),
            value: shown(worth, format::usd),
            cost: shown(balance.entry_value, format::usd),
            pnl: shown(made, format::signed_usd),
            tone: made.map(Tone::of),
            worth,
        }
    }
}

/// The rows of one account's `balances` priced at `prices`.
#[cfg(test)]
pub(super) fn holdings_of(
    balances: &Balances,
    prices: Option<&Prices>,
    hide_small: bool,
) -> Vec<Holding> {
    holdings_of_all(&[(None, balances, prices)], hide_small)
}

/// One account's part of the holdings: its mark (none when the widget
/// shows one account), what it holds, and its exchange's prices.
pub(super) type Held<'a> = (Option<Rc<Mark>>, &'a Balances, Option<&'a Prices>);

/// The rows of what `parts` hold, each account's assets priced at its own
/// exchange's prices, an asset of an account once: the most valuable
/// first, whichever account each is of, then those that are not priced, in
/// the accounts' and their exchanges' order. `hide_small` leaves out the
/// priced ones worth less than [`SMALL`].
pub(super) fn holdings_of_all(parts: &[Held<'_>], hide_small: bool) -> Vec<Holding> {
    let mut holdings: Vec<Holding> = Vec::new();
    for (mark, balances, prices) in parts {
        for balance in &balances.0 {
            let holding = Holding::of(balance, *prices, mark.as_ref());
            if !holdings.iter().any(|kept| kept.key == holding.key) {
                holdings.push(holding);
            }
        }
    }
    if hide_small {
        holdings.retain(|holding| holding.worth.is_none_or(|worth| worth >= SMALL));
    }
    // Stable: the unpriced keep the exchange's order.
    holdings.sort_by(|a, b| match (a.worth, b.worth) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    holdings
}

/// The report `each` makes of each of `accounts`, followed as they change.
fn watched<T: 'static>(
    scope: SessionScope,
    portfolio: &Portfolio,
    accounts: Memo<Vec<Account>>,
    each: fn(&Portfolio, &Account) -> Result<Watch<T>, Unavailable>,
) -> Followed<T> {
    let portfolio = portfolio.clone();
    follow_all(scope, accounts, move |account| each(&portfolio, account))
}

/// `account`'s report among those `arrived`.
fn found<'a, T>(arrived: &'a [(Account, Rc<T>)], account: &Account) -> Option<&'a T> {
    arrived
        .iter()
        .find(|(of, _)| of == account)
        .map(|(_, report)| &**report)
}

/// An Account Summary's session state: its accounts' reports, and what its
/// view shows of them (its watches live in the effects `follow_all`
/// starts).
pub(crate) struct AccountSummarySession {
    /// The exchanges that report accounts, which its accounts' rows list.
    pub(super) portfolio: Portfolio,
    /// The margin report of each account the settings name, which the
    /// frame speaks for.
    pub(super) margin: Followed<MarginAccount>,
    /// Their balances, which the balances' own note speaks for.
    pub(super) balances: Followed<Balances>,
    /// The overview, updated while the widget is visible.
    pub(super) overview: Signal<Overview>,
    /// The holdings, in the table's order, updated while it is visible.
    pub(super) holdings: Signal<Shared<Vec<Holding>>>,
}

impl AccountSummarySession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        portfolio: Portfolio,
        settings: Signal<AccountSummarySettings>,
    ) -> Self {
        let accounts = computed(move || settings.with(|settings| settings.accounts().to_vec()));
        let hide_small = computed(move || settings.with(AccountSummarySettings::small_hidden));
        let margin = watched(scope, &portfolio, accounts, Portfolio::watch_margin);
        let balances = watched(scope, &portfolio, accounts, Portfolio::watch_balances);
        let prices = watched(scope, &portfolio, accounts, Portfolio::watch_prices);
        let performance = watched(scope, &portfolio, accounts, Portfolio::watch_performance);
        let staking = watched(scope, &portfolio, accounts, Portfolio::watch_staking);
        let vaults = watched(scope, &portfolio, accounts, Portfolio::watch_vaults);
        let fees = watched(scope, &portfolio, accounts, Portfolio::watch_fees);
        let overview = signal(Overview::of(Reports::default()));
        let holdings = signal(Shared::new(Vec::new()));
        {
            let portfolio = portfolio.clone();
            scope.while_visible(move || {
                // An account counts as soon as its own reports arrive, none
                // waiting for another's: the frame says how many show
                // meanwhile.
                effect(move || {
                    let listed = accounts.get();
                    let margin = margin.arrived().unwrap_or_default();
                    let balances = balances.arrived().unwrap_or_default();
                    let prices = prices.arrived().unwrap_or_default();
                    let performance = performance.arrived().unwrap_or_default();
                    let staking = staking.arrived().unwrap_or_default();
                    let vaults = vaults.arrived().unwrap_or_default();
                    let fees = fees.arrived().unwrap_or_default();
                    let each: Vec<Reports<'_>> = listed
                        .iter()
                        .map(|account| Reports {
                            margin: found(&margin, account),
                            balances: found(&balances, account),
                            prices: found(&prices, account),
                            performance: found(&performance, account),
                            staking: found(&staking, account),
                            vaults: found(&vaults, account),
                            fees: found(&fees, account),
                        })
                        .collect();
                    overview.set(match each.is_empty() {
                        true => Overview::of(Reports::default()),
                        false => Overview::of_accounts(&each),
                    });
                });
                let portfolio = portfolio.clone();
                effect(move || {
                    let listed = accounts.get();
                    // An asset says whose it is once there are several
                    // accounts to be of.
                    let several = listed.len() > 1;
                    let prices = prices.arrived().unwrap_or_default();
                    let balances = balances.arrived().unwrap_or_default();
                    let parts: Vec<Held<'_>> = balances
                        .iter()
                        .map(|(account, held)| {
                            (
                                Mark::when(several, &portfolio, account),
                                &**held,
                                found(&prices, account),
                            )
                        })
                        .collect();
                    holdings.set(Shared::new(holdings_of_all(&parts, hide_small.get())));
                });
            });
        }
        Self {
            portfolio,
            margin,
            balances,
            overview,
            holdings,
        }
    }
}
