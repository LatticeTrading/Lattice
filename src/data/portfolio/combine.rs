//! Several accounts' reports as one: what a widget that shows accounts
//! together plots and adds up, whichever venues they are on, and what a
//! venue whose address holds several accounts makes of them.
//!
//! A history ([`Performance::combined`]) is combined window by window: the
//! accounts' series are added up at every time any of them was sampled.
//!
//! - Between its samples an account counts as what it last was: venues
//!   sample at times of their own (one on the hour, another whenever it
//!   pleases), and what an account is worth does not go away between two
//!   samples.
//! - Before its first sample it counts as what it first was, since two
//!   venues' windows of one period do not start on the same instant; but
//!   over all time, where a history starts when the account does, it counts
//!   as nothing until then: it was worth nothing and had made nothing.
//! - What the accounts traded is added up only when every one of them
//!   reports it: a sum that leaves one out would read as the whole.
//! - A window some of them do not report is the others' alone.
//!
//! The accounts of one venue's address add up too (a main account and its
//! sub-accounts):
//!
//! - margin accounts ([`MarginAccount::combined`]): every figure added up,
//!   one that some of them do not report left as not reported, and every
//!   position listed. The margin ratio is the highest of theirs, not that
//!   of the sums: each account is liquidated on its own;
//! - balances ([`Balances::combined`]): an asset once, what each holds of it
//!   together (within one venue, where a name is one asset);
//! - trades ([`Fills::combined`]): all of them, newest first.
//!
//! One account combined is itself. A widget that lists what accounts on
//! several venues hold (their positions, balances and trades) keeps them
//! apart, each marked with its account, and adds up their figures alone.

use super::model::{
    Balance, Balances, Fills, MarginAccount, Performance, Period, Point, Scope, Window,
};

impl MarginAccount {
    /// The margin accounts `parts` as one. Nothing of none.
    pub fn combined<'a>(parts: impl IntoIterator<Item = &'a Self>) -> Self {
        let parts: Vec<&Self> = parts.into_iter().collect();
        match parts[..] {
            [] => return Self::default(),
            [only] => return only.clone(),
            _ => {}
        }
        // A sum that leaves an account out would read as the whole.
        let sum = |read: fn(&Self) -> Option<f64>| -> Option<f64> {
            parts.iter().map(|part| read(part)).sum()
        };
        Self {
            // As old as the oldest of them.
            time_ms: parts
                .iter()
                .filter_map(|part| part.time_ms)
                .min_by(f64::total_cmp),
            equity: sum(|part| part.equity),
            unrealized_pnl: sum(|part| part.unrealized_pnl),
            position_value: sum(|part| part.position_value),
            margin_used: sum(|part| part.margin_used),
            maintenance_margin: sum(|part| part.maintenance_margin),
            withdrawable: sum(|part| part.withdrawable),
            cross_equity: sum(|part| part.cross_equity),
            funding: sum(|part| part.funding),
            nearest_margin_ratio: parts
                .iter()
                .filter_map(|part| part.margin_ratio())
                .max_by(f64::total_cmp),
            positions: parts
                .iter()
                .flat_map(|part| part.positions.iter().cloned())
                .collect(),
        }
    }
}

impl Balances {
    /// The balances `parts` of one venue as one: an asset once, in the
    /// order it is first met, what each holds of it together. What it cost
    /// is known when every holder of it says so.
    pub fn combined<'a>(parts: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut assets: Vec<Balance> = Vec::new();
        for balance in parts.into_iter().flat_map(|part| &part.0) {
            match assets.iter_mut().find(|kept| kept.asset == balance.asset) {
                Some(kept) => {
                    kept.total += balance.total;
                    kept.held += balance.held;
                    kept.entry_value = kept
                        .entry_value
                        .zip(balance.entry_value)
                        .map(|(kept, more)| kept + more);
                }
                None => assets.push(balance.clone()),
            }
        }
        Self(assets)
    }
}

impl Fills {
    /// The trades `parts` as one list, newest first.
    pub fn combined<'a>(parts: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut fills: Vec<_> = parts
            .into_iter()
            .flat_map(|part| part.0.iter().cloned())
            .collect();
        fills.sort_by(|a, b| b.time_ms.total_cmp(&a.time_ms));
        Self(fills)
    }
}

/// What a series counts as before its first sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Before {
    /// What it first was.
    First,
    /// Nothing: there was no account yet.
    Nothing,
}

impl Performance {
    /// The histories `parts` as one: each window every time any of them
    /// was sampled, the accounts added up. Nothing of none.
    pub fn combined<'a>(parts: impl IntoIterator<Item = &'a Self>) -> Self {
        let parts: Vec<&Self> = parts.into_iter().collect();
        if let [only] = parts[..] {
            return only.clone();
        }
        let mut windows = Vec::new();
        for scope in Scope::ALL {
            for period in Period::ALL {
                let reported: Vec<&Window> = parts
                    .iter()
                    .filter_map(|part| part.window(period, scope))
                    .collect();
                if !reported.is_empty() {
                    windows.push(combined_window(period, scope, &reported));
                }
            }
        }
        Self(windows)
    }
}

/// The windows `parts`, each of `period` and `scope`, as one.
fn combined_window(period: Period, scope: Scope, parts: &[&Window]) -> Window {
    let series = |read: fn(&Window) -> &[Point]| -> Vec<&[Point]> {
        parts.iter().map(|window| read(window)).collect()
    };
    let before = match period {
        Period::AllTime => Before::Nothing,
        Period::Day | Period::Week | Period::Month => Before::First,
    };
    Window {
        period,
        scope,
        value: summed(&series(|window| &window.value), before),
        pnl: summed(&series(|window| &window.pnl), before),
        volume: parts.iter().map(|window| window.volume).sum(),
    }
}

/// `series` (each oldest first) added up at every time any of them has a
/// sample, each counting as its latest sample by then, and as `before` says
/// until its first. A series with no sample adds nothing.
fn summed(series: &[&[Point]], before: Before) -> Vec<Point> {
    let series: Vec<&[Point]> = series
        .iter()
        .copied()
        .filter(|points| !points.is_empty())
        .collect();
    match series[..] {
        [] => return Vec::new(),
        [only] => return only.to_vec(),
        _ => {}
    }
    let mut times: Vec<f64> = series
        .iter()
        .flat_map(|points| points.iter().map(|point| point.time_ms))
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    // How many of each series' samples are at or before the time summed.
    let mut passed = vec![0_usize; series.len()];
    times
        .into_iter()
        .map(|time_ms| {
            let value = series
                .iter()
                .zip(&mut passed)
                .map(|(points, passed)| {
                    while *passed < points.len() && points[*passed].time_ms <= time_ms {
                        *passed += 1;
                    }
                    match (*passed, before) {
                        (0, Before::First) => points[0].value,
                        (0, Before::Nothing) => 0.0,
                        (passed, _) => points[passed - 1].value,
                    }
                })
                .sum();
            Point { time_ms, value }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::portfolio::{Direction, Fill, Position, TradeSide};

    fn position(instrument: &str, value: f64) -> Position {
        Position {
            instrument: instrument.to_owned(),
            direction: Direction::Long,
            size: 1.0,
            entry_price: None,
            mark_price: None,
            value: Some(value),
            unrealized_pnl: None,
            return_on_equity: None,
            liquidation_price: None,
            margin_used: None,
            leverage: None,
        }
    }

    #[test]
    fn margin_accounts_add_up_and_keep_the_ratio_nearest_liquidation() {
        let safe = MarginAccount {
            equity: Some(10_000.0),
            unrealized_pnl: Some(50.0),
            position_value: Some(4_000.0),
            margin_used: Some(400.0),
            maintenance_margin: Some(100.0),
            withdrawable: Some(9_000.0),
            cross_equity: Some(10_000.0),
            funding: Some(-2.0),
            time_ms: Some(2_000.0),
            positions: vec![position("BTC", 4_000.0)],
            ..MarginAccount::default()
        };
        let strained = MarginAccount {
            equity: Some(1_000.0),
            unrealized_pnl: Some(-300.0),
            position_value: Some(9_000.0),
            margin_used: Some(900.0),
            maintenance_margin: Some(800.0),
            // Not reported: neither is the sum.
            withdrawable: None,
            cross_equity: Some(1_000.0),
            funding: Some(1.5),
            time_ms: Some(1_000.0),
            positions: vec![position("BTC", 6_000.0), position("ETH", 3_000.0)],
            ..MarginAccount::default()
        };
        let both = MarginAccount::combined([&safe, &strained]);
        assert_eq!(both.equity, Some(11_000.0));
        assert_eq!(both.unrealized_pnl, Some(-250.0));
        assert_eq!(both.position_value, Some(13_000.0));
        assert_eq!(both.maintenance_margin, Some(900.0));
        assert_eq!(both.funding, Some(-0.5));
        assert_eq!(both.withdrawable, None);
        assert_eq!(both.time_ms, Some(1_000.0));
        // Every position, each account's own.
        let held: Vec<&str> = both
            .positions
            .iter()
            .map(|position| position.instrument.as_str())
            .collect();
        assert_eq!(held, ["BTC", "BTC", "ETH"]);
        // 900 of 11,000 would read as safe; one of them is at 80%.
        assert_eq!(both.margin_ratio(), Some(0.8));
        assert_eq!(both.liquidation_buffer(), Some(10_100.0));
        assert!((both.leverage().unwrap() - 13_000.0 / 11_000.0).abs() < 1e-12);

        // One is itself, and none is nothing.
        assert_eq!(MarginAccount::combined([&safe]), safe);
        assert_eq!(MarginAccount::combined([]), MarginAccount::default());
    }

    #[test]
    fn balances_and_trades_of_one_venue_are_listed_once_and_in_order() {
        let balance = |asset: &str, total: f64, held: f64, entry_value: Option<f64>| Balance {
            asset: asset.to_owned(),
            total,
            held,
            entry_value,
        };
        let main = Balances(vec![
            balance("USDC", 100.0, 10.0, Some(100.0)),
            balance("LIT", 5.0, 0.0, None),
        ]);
        let sub = Balances(vec![
            balance("LIT", 2.5, 1.0, Some(4.0)),
            balance("ETH", 1.0, 0.0, Some(1_700.0)),
            balance("USDC", 50.0, 0.0, Some(50.0)),
        ]);
        let both = Balances::combined([&main, &sub]);
        assert_eq!(
            both,
            Balances(vec![
                balance("USDC", 150.0, 10.0, Some(150.0)),
                // What it cost is not known of all of it.
                balance("LIT", 7.5, 1.0, None),
                balance("ETH", 1.0, 0.0, Some(1_700.0)),
            ])
        );

        let fill = |id: &str, time_ms: f64| Fill {
            id: id.to_owned(),
            time_ms,
            instrument: "BTC".to_owned(),
            side: TradeSide::Buy,
            action: None,
            price: 1.0,
            size: 1.0,
            fee: None,
            fee_asset: None,
            closed_pnl: None,
            taker: None,
        };
        let trades = Fills::combined([
            &Fills(vec![fill("c", 3_000.0), fill("a", 1_000.0)]),
            &Fills(vec![fill("d", 4_000.0), fill("b", 2_000.0)]),
        ]);
        let ids: Vec<&str> = trades.0.iter().map(|fill| fill.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "b", "a"]);
    }

    fn points(samples: &[(f64, f64)]) -> Vec<Point> {
        samples
            .iter()
            .map(|(time_ms, value)| Point {
                time_ms: *time_ms,
                value: *value,
            })
            .collect()
    }

    fn window(
        period: Period,
        scope: Scope,
        value: &[(f64, f64)],
        pnl: &[(f64, f64)],
        volume: Option<f64>,
    ) -> Window {
        Window {
            period,
            scope,
            value: points(value),
            pnl: points(pnl),
            volume,
        }
    }

    #[test]
    fn histories_add_up_at_every_time_any_of_them_was_sampled() {
        // Two venues that sample a day at times of their own.
        let hourly = Performance(vec![window(
            Period::Day,
            Scope::Total,
            &[(1_000.0, 100.0), (3_000.0, 110.0), (5_000.0, 120.0)],
            &[(1_000.0, 0.0), (3_000.0, 10.0), (5_000.0, 20.0)],
            Some(500.0),
        )]);
        let ragged = Performance(vec![window(
            Period::Day,
            Scope::Total,
            &[(2_000.0, 50.0), (3_000.0, 45.0), (6_000.0, 40.0)],
            &[(2_000.0, 1.0), (3_000.0, -4.0), (6_000.0, -9.0)],
            Some(250.5),
        )]);
        let both = Performance::combined([&hourly, &ragged]);
        let day = both.window(Period::Day, Scope::Total).unwrap();
        // Before its first sample an account is worth what it first was,
        // between two what it last was.
        assert_eq!(
            day.value,
            points(&[
                (1_000.0, 150.0),
                (2_000.0, 150.0),
                (3_000.0, 155.0),
                (5_000.0, 165.0),
                (6_000.0, 160.0),
            ])
        );
        assert_eq!(
            day.pnl,
            points(&[
                (1_000.0, 1.0),
                (2_000.0, 1.0),
                (3_000.0, 6.0),
                (5_000.0, 16.0),
                (6_000.0, 11.0),
            ])
        );
        // What the day made is what each made, together.
        let made = day.pnl.last().unwrap().value - day.pnl[0].value;
        assert_eq!(made, 20.0 + -10.0);
        assert_eq!(day.volume, Some(750.5));
        // The order they are given in changes nothing.
        assert_eq!(Performance::combined([&ragged, &hourly]), both);
    }

    #[test]
    fn over_all_time_an_account_counts_as_nothing_until_it_starts() {
        let old = Performance(vec![window(
            Period::AllTime,
            Scope::Total,
            &[(1_000.0, 100.0), (9_000.0, 300.0)],
            &[(1_000.0, 0.0), (9_000.0, 200.0)],
            Some(10.0),
        )]);
        let new = Performance(vec![window(
            Period::AllTime,
            Scope::Total,
            &[(5_000.0, 50.0), (9_000.0, 60.0)],
            // Its first sample is a day in: it had lost 3 by then.
            &[(5_000.0, -3.0), (9_000.0, 10.0)],
            Some(5.0),
        )]);
        let both = Performance::combined([&old, &new]);
        let all = both.window(Period::AllTime, Scope::Total).unwrap();
        assert_eq!(
            all.value,
            points(&[(1_000.0, 100.0), (5_000.0, 150.0), (9_000.0, 360.0)])
        );
        assert_eq!(
            all.pnl,
            points(&[(1_000.0, 0.0), (5_000.0, -3.0), (9_000.0, 210.0)])
        );
    }

    #[test]
    fn one_history_is_itself_and_what_some_do_not_report_is_the_others() {
        let whole = Performance(vec![
            window(
                Period::Week,
                Scope::Total,
                &[(1_000.0, 10.0)],
                &[(1_000.0, 0.0)],
                Some(1.0),
            ),
            window(
                Period::Week,
                Scope::Perps,
                &[(1_000.0, 4.0)],
                &[(1_000.0, 0.0)],
                Some(1.0),
            ),
        ]);
        assert_eq!(Performance::combined([&whole]), whole);
        assert_eq!(Performance::combined([]), Performance::default());

        // One that reports the whole account alone, without its volume,
        // and with nothing sampled yet.
        let partial = Performance(vec![window(Period::Week, Scope::Total, &[], &[], None)]);
        let both = Performance::combined([&whole, &partial]);
        let total = both.window(Period::Week, Scope::Total).unwrap();
        assert_eq!(total.value, points(&[(1_000.0, 10.0)]));
        // A sum without one account's volume would read as the whole.
        assert_eq!(total.volume, None);
        let perps = both.window(Period::Week, Scope::Perps).unwrap();
        assert_eq!(perps.value, points(&[(1_000.0, 4.0)]));
        assert_eq!(perps.volume, Some(1.0));
        assert!(both.window(Period::Day, Scope::Total).is_none());
    }
}
