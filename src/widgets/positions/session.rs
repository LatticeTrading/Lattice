//! The Positions' session ([`PositionsSession`]): the margin report of each
//! account its settings name, and the rows its view shows of them together
//! (the widget's data, described in the parent module).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::settings::PositionsSettings;
use crate::{
    data::{
        Portfolio, SessionScope,
        portfolio::{Direction, MarginAccount, Position},
    },
    format,
    widgets::common::{
        Shared,
        accounts::{AccountSettings, Mark, NONE, Reports, Tone, follow_all, shown},
    },
};

/// One position's row, as its cells show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    /// What tells the row apart: its instrument and which way it faces,
    /// after its account when the widget shows several.
    pub key: Rc<str>,
    /// Whose it is, when the widget shows several accounts.
    pub owner: Option<Rc<Mark>>,
    pub instrument: String,
    pub direction: Direction,
    /// Its leverage setting (`20x Isolated`), empty when not reported.
    pub leverage: String,
    pub size: String,
    pub value: String,
    pub entry: String,
    pub mark: String,
    /// Its unrealized profit and loss, with its return on equity when
    /// reported (`-$0.01 (-0.27%)`).
    pub pnl: String,
    /// What colors [`Self::pnl`]; `None` when it is not reported.
    pub tone: Option<Tone>,
    pub liquidation: String,
    pub margin: String,
}

impl Row {
    fn of(position: &Position, owner: Option<Rc<Mark>>, key: Rc<str>) -> Self {
        let pnl = match (position.unrealized_pnl, position.return_on_equity) {
            (Some(pnl), Some(return_on_equity)) => format!(
                "{} ({})",
                format::signed_usd(pnl),
                format::signed_percent(return_on_equity)
            ),
            (Some(pnl), None) => format::signed_usd(pnl),
            (None, _) => NONE.to_owned(),
        };
        Self {
            key,
            owner,
            instrument: position.instrument.clone(),
            direction: position.direction,
            leverage: position.leverage.map_or_else(String::new, |leverage| {
                format!(
                    "{}x {}",
                    format::quantity(leverage.value),
                    leverage.mode.label()
                )
            }),
            size: format::quantity(position.size),
            value: shown(position.value, format::usd),
            entry: shown(position.entry_price, format::price_text),
            mark: shown(position.mark_price, format::price_text),
            pnl,
            tone: position.unrealized_pnl.map(Tone::of),
            liquidation: shown(position.liquidation_price, format::price_text),
            margin: shown(position.margin_used, format::usd),
        }
    }
}

/// The rows of one account's `positions`, in the table's order.
#[cfg(test)]
pub(super) fn rows_of(positions: &[Position]) -> Vec<Row> {
    rows_of_all(&[(None, positions)])
}

/// The rows of the positions `parts` hold, each account's with its mark
/// (none when the widget shows one account), in the table's order: the
/// position worth the most first, those of unknown worth last, then by
/// instrument, whichever account each is of.
pub(super) fn rows_of_all(parts: &[(Option<Rc<Mark>>, &[Position])]) -> Vec<Row> {
    let mut ordered: Vec<(Option<&Rc<Mark>>, &Position)> = parts
        .iter()
        .flat_map(|(mark, positions)| {
            positions
                .iter()
                .map(move |position| (mark.as_ref(), position))
        })
        .collect();
    ordered.sort_by(|(_, a), (_, b)| {
        let worth = |position: &Position| position.value.unwrap_or(f64::NEG_INFINITY);
        worth(b)
            .total_cmp(&worth(a))
            .then_with(|| format::compare_text(&a.instrument, &b.instrument))
    });
    let mut rows: Vec<Row> = Vec::with_capacity(ordered.len());
    for (mark, position) in ordered {
        let own = format!("{}\n{}", position.instrument, position.direction.label());
        // An address's accounts on one exchange are one account here, and
        // each may hold the market: the second is a row of its own.
        let mut key = Mark::key(mark, &own);
        let mut nth = 1;
        while rows.iter().any(|row| row.key == key) {
            nth += 1;
            key = Mark::key(mark, &format!("{own}\n{nth}"));
        }
        rows.push(Row::of(position, mark.cloned(), key));
    }
    rows
}

/// A Positions widget's session state: its accounts' reports, and what its
/// view shows of them (its watches live in the effect `follow_all`
/// starts).
pub(crate) struct PositionsSession {
    /// The exchanges that report accounts, which its account's rows list.
    pub(super) portfolio: Portfolio,
    /// The margin report of each account the settings name, and where
    /// they stand as one.
    pub(super) reports: Reports<MarginAccount>,
    /// The rows, in the table's order, updated while the widget is visible.
    pub(super) rows: Signal<Shared<Vec<Row>>>,
}

impl PositionsSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        portfolio: Portfolio,
        settings: Signal<PositionsSettings>,
    ) -> Self {
        let accounts = computed(move || settings.with(|settings| settings.accounts().to_vec()));
        let reports = {
            let portfolio = portfolio.clone();
            follow_all(scope, accounts, move |account| {
                portfolio.watch_margin(account)
            })
        };
        let rows = signal(Shared::new(Vec::new()));
        {
            let portfolio = portfolio.clone();
            scope.while_visible(move || {
                let portfolio = portfolio.clone();
                effect(move || {
                    // A position says whose it is once there are several
                    // accounts to be of.
                    let several = accounts.with(|accounts| accounts.len() > 1);
                    let arrived = reports.arrived().unwrap_or_default();
                    let parts: Vec<(Option<Rc<Mark>>, &[Position])> = arrived
                        .iter()
                        .map(|(account, margin)| {
                            (
                                Mark::when(several, &portfolio, account),
                                &margin.positions[..],
                            )
                        })
                        .collect();
                    rows.set(Shared::new(rows_of_all(&parts)));
                });
            });
        }
        Self {
            portfolio,
            reports,
            rows,
        }
    }
}
