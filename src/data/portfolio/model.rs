//! What an account is said to hold and have made, whatever its venue.
//!
//! These are the shapes widgets show; each venue's integration fills them
//! from its own wire format. A member a venue does not report is `None`,
//! never a guess, so a widget shows a dash for it.
//!
//! Figures are `f64`, in the account's quote currency (US dollars on every
//! venue so far) unless a member says otherwise: they are sums, ratios and
//! plotted series, shown rounded. Nothing here is a price to trade at.

use std::collections::HashMap;

/// Which way a position faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Long => "Long",
            Self::Short => "Short",
        }
    }
}

/// What backs a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarginMode {
    /// The whole account's margin.
    Cross,
    /// Margin set aside for this position alone.
    Isolated,
}

impl MarginMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cross => "Cross",
            Self::Isolated => "Isolated",
        }
    }
}

/// A position's leverage setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leverage {
    /// How many times its margin the position may be worth (`20.0`).
    pub value: f64,
    pub mode: MarginMode,
}

/// One open position.
#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    /// The venue's name for what is held (`ETH`, `xyz:XYZ100`).
    pub instrument: String,
    pub direction: Direction,
    /// How much is held, in the instrument's units; never negative
    /// ([`Self::direction`] says which way).
    pub size: f64,
    /// The average price it was entered at.
    pub entry_price: Option<f64>,
    /// The price it is valued at now.
    pub mark_price: Option<f64>,
    /// What it is worth now: its size at the mark price.
    pub value: Option<f64>,
    /// What closing it now would make, before fees.
    pub unrealized_pnl: Option<f64>,
    /// [`Self::unrealized_pnl`] over the margin it was opened with, as a
    /// fraction (`-0.0027` is -0.27%).
    pub return_on_equity: Option<f64>,
    /// The mark price it would be liquidated at; `None` also when there is
    /// none (a position the account's margin covers at any price).
    pub liquidation_price: Option<f64>,
    /// The margin it holds.
    pub margin_used: Option<f64>,
    pub leverage: Option<Leverage>,
}

impl Position {
    /// How much is held, signed: above zero long, below zero short.
    fn signed_size(&self) -> f64 {
        match self.direction {
            Direction::Long => self.size,
            Direction::Short => -self.size,
        }
    }

    /// The margin its return is measured against. Venues differ in what
    /// that is (the margin it was opened with, the margin it holds now), so
    /// it is what the venue's own figures say: its profit over its return.
    /// Where they say too little to tell (a return too near zero to divide
    /// by, or none), it is the margin it was opened with: what it was worth
    /// at its entry over its leverage.
    fn return_basis(&self) -> Option<f64> {
        /// The smallest return a profit is divided by.
        const TELLING: f64 = 1e-4;
        let sound = |basis: f64| (basis.is_finite() && basis > 0.0).then_some(basis);
        let reported = self
            .unrealized_pnl
            .zip(self.return_on_equity)
            .filter(|(_, return_on_equity)| return_on_equity.abs() >= TELLING)
            .and_then(|(pnl, return_on_equity)| sound(pnl / return_on_equity));
        reported.or_else(|| {
            let (entry, leverage) = (self.entry_price?, self.leverage?);
            sound(self.size * entry / leverage.value)
        })
    }

    /// The position as it stands with its instrument priced at `price`, for
    /// a price heard since its venue made the report (its mark, or what its
    /// book says: [`PriceKind`](crate::data::prices::PriceKind)): what it is
    /// worth there,
    /// what closing it there would make, and its return on that.
    ///
    /// The report is where it starts from: its profit moves by what the
    /// price is away from the reported one, so at the report's own price it
    /// is the report, whatever its venue counts in a profit. Without a
    /// reported profit to start from it is what the price has moved since
    /// the entry. What a price alone does not say (its liquidation price,
    /// the margin it holds) stays as reported. For what is no price above
    /// zero, the position as reported.
    #[must_use]
    pub fn priced_at(&self, price: f64) -> Self {
        if !price.is_finite() || price <= 0.0 {
            return self.clone();
        }
        let signed = self.signed_size();
        let pnl = match (self.unrealized_pnl, self.mark_price) {
            (Some(pnl), Some(mark)) => Some(pnl + (price - mark) * signed),
            _ => self.entry_price.map(|entry| (price - entry) * signed),
        };
        Self {
            mark_price: Some(price),
            value: Some(self.size * price),
            unrealized_pnl: pnl,
            return_on_equity: pnl.zip(self.return_basis()).map(|(pnl, basis)| pnl / basis),
            ..self.clone()
        }
    }
}

/// An account's margin trading as it stands: what backs its positions, and
/// the positions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarginAccount {
    /// When the venue made the report, in Unix milliseconds.
    pub time_ms: Option<f64>,
    /// What the margin account is worth: its collateral with the open
    /// positions' unrealized profit and loss.
    pub equity: Option<f64>,
    /// What closing every position now would make, before fees.
    pub unrealized_pnl: Option<f64>,
    /// What the open positions are worth together, long and short alike.
    pub position_value: Option<f64>,
    /// The margin the open positions hold.
    pub margin_used: Option<f64>,
    /// The margin under which the cross-margined positions are liquidated
    /// (each isolated position has its own, in its liquidation price).
    pub maintenance_margin: Option<f64>,
    /// What can be taken out now.
    pub withdrawable: Option<f64>,
    /// What the cross margin is worth: the part of [`Self::equity`] that
    /// backs every cross position together, and that
    /// [`Self::maintenance_margin`] is measured against.
    pub cross_equity: Option<f64>,
    /// What funding has paid the open positions since each was opened,
    /// together; below zero, what they have paid. `Some(0.0)` with none
    /// open.
    pub funding: Option<f64>,
    /// Of several accounts added up ([`Self::combined`]): the highest of
    /// their margin ratios. Each is liquidated on its own, so the ratio of
    /// their sums would hide the one nearest it. `None` for one account.
    pub nearest_margin_ratio: Option<f64>,
    /// The open positions, in the venue's order.
    pub positions: Vec<Position>,
}

impl MarginAccount {
    /// How many times its equity the account's positions are worth; `None`
    /// without either figure, or with no equity to measure against.
    pub fn leverage(&self) -> Option<f64> {
        let (value, equity) = (self.position_value?, self.equity?);
        (equity > 0.0).then(|| value / equity)
    }

    /// How much of the cross margin the maintenance margin takes, as a
    /// fraction: the cross positions are liquidated at `1.0`. `None`
    /// without either figure, or with no cross margin to measure against.
    /// Of several accounts added up, that of the one nearest liquidation
    /// ([`Self::nearest_margin_ratio`]).
    pub fn margin_ratio(&self) -> Option<f64> {
        self.nearest_margin_ratio.or_else(|| {
            let (maintenance, equity) = (self.maintenance_margin?, self.cross_equity?);
            (equity > 0.0).then(|| maintenance / equity)
        })
    }

    /// How much the cross margin can lose before the cross positions are
    /// liquidated.
    pub fn liquidation_buffer(&self) -> Option<f64> {
        Some(self.cross_equity? - self.maintenance_margin?)
    }
}

/// One asset an account holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Balance {
    /// The venue's name for the asset (`USDC`, `HYPE`).
    pub asset: String,
    /// How much is held, in the asset's units.
    pub total: f64,
    /// How much of it open orders hold.
    pub held: f64,
    /// What it cost, in the quote currency.
    pub entry_value: Option<f64>,
}

impl Balance {
    /// What is free to trade or move: the total less what orders hold.
    pub fn available(&self) -> f64 {
        (self.total - self.held).max(0.0)
    }
}

/// The assets an account holds, in the venue's order; those it holds none
/// of are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Balances(pub Vec<Balance>);

/// A stretch of an account's history a venue reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    Day,
    Week,
    Month,
    AllTime,
}

impl Period {
    /// Every period, shortest first.
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::AllTime];
}

/// Which part of an account a history covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Everything the account holds on the venue.
    Total,
    /// Its perpetuals trading alone.
    Perps,
}

impl Scope {
    pub const ALL: [Self; 2] = [Self::Total, Self::Perps];
}

/// A figure at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// When, in Unix milliseconds.
    pub time_ms: f64,
    pub value: f64,
}

/// One period of one part of an account: what it was worth and what it
/// made, sampled over the period, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub period: Period,
    pub scope: Scope,
    /// What the account was worth.
    pub value: Vec<Point>,
    /// What it had made since the period began (the first point is where
    /// the count starts).
    pub pnl: Vec<Point>,
    /// What it traded in the period.
    pub volume: Option<f64>,
}

/// What a venue's assets are worth now, in the quote currency, by the name
/// the venue gives each (`HYPE`); the quote currency itself is `1.0`. An
/// asset with no market to price it by is left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prices(pub HashMap<String, f64>);

impl Prices {
    /// What one unit of `asset` is worth, when it is priced.
    pub fn of(&self, asset: &str) -> Option<f64> {
        self.0.get(asset).copied()
    }
}

/// What an account has staked: the venue's token, delegated to validators
/// or on its way in or out.
#[derive(Clone, Debug, PartialEq)]
pub struct Staking {
    /// The token staked (`HYPE`), as [`Prices`] names it.
    pub asset: String,
    /// How much is delegated, earning rewards.
    pub delegated: f64,
    /// How much is staked but delegated to no validator.
    pub undelegated: f64,
    /// How much is on its way back to the account's balances.
    pub pending_withdrawal: f64,
}

impl Staking {
    /// Everything staked, however it stands.
    pub fn total(&self) -> f64 {
        self.delegated + self.undelegated + self.pending_withdrawal
    }
}

/// An account's share of one vault.
#[derive(Clone, Debug, PartialEq)]
pub struct VaultShare {
    /// The vault's address.
    pub vault: String,
    /// What the share is worth, in the quote currency.
    pub equity: f64,
    /// Until when the share cannot be withdrawn, in Unix milliseconds.
    pub locked_until_ms: Option<f64>,
}

/// The vaults an account has a share of, in the venue's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vaults(pub Vec<VaultShare>);

impl Vaults {
    /// What the shares are worth together.
    pub fn equity(&self) -> f64 {
        self.0.iter().map(|share| share.equity).sum()
    }
}

/// What an account pays to trade, and the volume its rates are set by.
/// Rates are fractions of what is traded (`0.00045` is 0.045%), every
/// discount the account has already taken off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fees {
    /// Perpetuals, taking liquidity.
    pub taker: Option<f64>,
    /// Perpetuals, adding liquidity.
    pub maker: Option<f64>,
    /// Spot, taking liquidity.
    pub spot_taker: Option<f64>,
    /// Spot, adding liquidity.
    pub spot_maker: Option<f64>,
    /// The volume the tier is counted on, over [`Self::volume_days`].
    pub volume: Option<f64>,
    pub volume_days: u32,
    /// The account's volume tier: `0` is the base rate.
    pub tier: Option<usize>,
    /// The volume the account's own tier starts at (`0.0` for the base).
    pub tier_floor: Option<f64>,
    /// The volume the next tier starts at; `None` at the top.
    pub next_tier: Option<f64>,
    /// What is taken off its rates for staking and referrals, as a
    /// fraction.
    pub discount: Option<f64>,
}

/// Which side of the book a trade was on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

impl TradeSide {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }
}

/// One of an account's trades: what one order filled at one time.
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    /// The venue's id of the trade, which tells two fills apart.
    pub id: String,
    /// When, in Unix milliseconds.
    pub time_ms: f64,
    /// The venue's name for what was traded (`ETH`, `PURR/USDC`).
    pub instrument: String,
    pub side: TradeSide,
    /// What the trade did, in the venue's words (`Open Long`, `Close
    /// Short`); `None` when not reported.
    pub action: Option<String>,
    /// The price it filled at (the average, for several fills together).
    pub price: f64,
    /// How much filled, in the instrument's units.
    pub size: f64,
    /// What it cost to trade, in [`Self::fee_asset`]; below zero, a rebate.
    pub fee: Option<f64>,
    pub fee_asset: Option<String>,
    /// What closing part of a position made, before fees; `None` for a
    /// trade that closed nothing.
    pub closed_pnl: Option<f64>,
    /// Whether it took liquidity (rather than adding it).
    pub taker: Option<bool>,
}

impl Fill {
    /// What it was worth: its size at its price.
    pub fn value(&self) -> f64 {
        self.size * self.price
    }
}

/// An account's latest trades, newest first: as many as the venue reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fills(pub Vec<Fill>);

/// An account's history: the windows its venue reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Performance(pub Vec<Window>);

impl Performance {
    /// The window of `period` and `scope`, when the venue reports it.
    pub fn window(&self, period: Period, scope: Scope) -> Option<&Window> {
        self.0
            .iter()
            .find(|window| window.period == period && window.scope == scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The long of Hyperliquid's documented reply: 0.0335 ETH entered at
    /// 2,986.3 and marked at 2,985.9, twenty times its own margin.
    fn long() -> Position {
        Position {
            instrument: "ETH".to_owned(),
            direction: Direction::Long,
            size: 0.0335,
            entry_price: Some(2_986.3),
            mark_price: Some(2_985.9),
            value: Some(100.027_65),
            unrealized_pnl: Some(-0.0134),
            return_on_equity: Some(-0.002_678_9),
            liquidation_price: Some(2_866.27),
            margin_used: Some(4.967_826),
            leverage: Some(Leverage {
                value: 20.0,
                mode: MarginMode::Isolated,
            }),
        }
    }

    fn near(figure: Option<f64>, wanted: f64) -> bool {
        figure.is_some_and(|figure| (figure - wanted).abs() < 1e-6)
    }

    #[test]
    fn a_position_is_priced_at_what_was_heard_since_its_report() {
        // The price rises: worth more, and what the rise makes of it over
        // the margin it was opened with (0.0335 at 2,986.3, over 20).
        let up = long().priced_at(3_000.0);
        assert_eq!(up.mark_price, Some(3_000.0));
        assert!(near(up.value, 100.5));
        assert!(near(up.unrealized_pnl, (3_000.0 - 2_986.3) * 0.0335));
        let opened_with = 0.0335 * 2_986.3 / 20.0;
        let roe = up.return_on_equity.unwrap();
        assert!((roe - 0.458_95 / opened_with).abs() < 1e-4, "{roe}");
        // What a price alone does not say stays as reported.
        assert_eq!(up.liquidation_price, Some(2_866.27));
        assert_eq!(up.margin_used, Some(4.967_826));
        assert_eq!((up.size, up.entry_price), (0.0335, Some(2_986.3)));

        // At the report's own mark it is the report.
        let same = long().priced_at(2_985.9);
        assert!(near(same.value, 100.027_65));
        assert!(near(same.unrealized_pnl, -0.0134));
        assert!((same.return_on_equity.unwrap() + 0.002_678_9).abs() < 1e-9);

        // A short gains as the price falls.
        let short = Position {
            direction: Direction::Short,
            size: 0.1,
            entry_price: Some(60_000.0),
            mark_price: Some(61_000.0),
            value: Some(6_100.0),
            unrealized_pnl: Some(-100.0),
            return_on_equity: Some(-0.0833),
            leverage: Some(Leverage {
                value: 5.0,
                mode: MarginMode::Cross,
            }),
            ..long()
        };
        let down = short.priced_at(59_000.0);
        assert!(near(down.value, 5_900.0));
        assert!(near(down.unrealized_pnl, 100.0));
        assert!((down.return_on_equity.unwrap() - 0.0833).abs() < 1e-9);

        // What is no price changes nothing.
        for no_price in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(long().priced_at(no_price), long(), "{no_price}");
        }
    }

    #[test]
    fn a_return_is_measured_as_its_venue_measures_it_or_against_the_opening_margin() {
        // A venue that measures against another margin (the profit is 40%
        // of it, not of the opening margin): its own measure is kept.
        let other = Position {
            unrealized_pnl: Some(2.0),
            return_on_equity: Some(0.4),
            ..long()
        };
        let moved = other.priced_at(2_985.9 + 1.0 / 0.0335);
        assert!(near(moved.unrealized_pnl, 3.0));
        assert!(near(moved.return_on_equity, 0.6));

        // A return too near zero to tell: the opening margin.
        let flat = Position {
            mark_price: Some(2_986.3),
            unrealized_pnl: Some(0.0),
            return_on_equity: Some(0.0),
            ..long()
        };
        let moved = flat.priced_at(3_000.0);
        let opened_with = 0.0335 * 2_986.3 / 20.0;
        assert!(near(moved.return_on_equity, 0.458_95 / opened_with));

        // No profit reported: what the price has moved since the entry.
        let bare = Position {
            unrealized_pnl: None,
            return_on_equity: None,
            mark_price: None,
            value: None,
            ..long()
        };
        let moved = bare.priced_at(3_000.0);
        assert!(near(moved.unrealized_pnl, 0.458_95));
        assert!(near(moved.return_on_equity, 0.458_95 / opened_with));
        // Nothing to measure it against, or to start from: not said.
        let unmeasured = Position {
            leverage: None,
            ..bare.clone()
        };
        assert_eq!(unmeasured.priced_at(3_000.0).return_on_equity, None);
        let unknown = Position {
            entry_price: None,
            ..bare
        };
        let moved = unknown.priced_at(3_000.0);
        assert_eq!((moved.unrealized_pnl, moved.return_on_equity), (None, None));
        assert!(near(moved.value, 100.5));
    }
}
