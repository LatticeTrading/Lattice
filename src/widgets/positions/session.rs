//! The Positions' session ([`PositionsSession`]): the margin report of the
//! account its settings name, and the rows its view shows (the widget's
//! data, described in the parent module).

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
        accounts::{AccountSettings, NONE, Report, Tone, follow, shown},
    },
};

/// One position's row, as its cells show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    /// What tells the row apart: its instrument and which way it faces.
    pub key: Rc<str>,
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
    fn of(position: &Position) -> Self {
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
            key: format!("{}\n{}", position.instrument, position.direction.label()).into(),
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

/// The rows of `positions`, in the table's order: the position worth the
/// most first, those of unknown worth last, then by instrument.
pub(super) fn rows_of(positions: &[Position]) -> Vec<Row> {
    let mut ordered: Vec<&Position> = positions.iter().collect();
    ordered.sort_by(|a, b| {
        let worth = |position: &Position| position.value.unwrap_or(f64::NEG_INFINITY);
        worth(b)
            .total_cmp(&worth(a))
            .then_with(|| format::compare_text(&a.instrument, &b.instrument))
    });
    ordered.into_iter().map(Row::of).collect()
}

/// A Positions widget's session state: its account's report, and what its
/// view shows of it (its watch lives in the effect `follow` starts).
pub(crate) struct PositionsSession {
    /// The exchanges that report accounts, which its account's rows list.
    pub(super) portfolio: Portfolio,
    /// The margin report of the account the settings name.
    pub(super) report: Report<MarginAccount>,
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
        let account = computed(move || settings.with(|settings| settings.account().cloned()));
        let report = {
            let portfolio = portfolio.clone();
            follow(scope, account, move |account| {
                portfolio.watch_margin(account)
            })
        };
        let rows = signal(Shared::new(Vec::new()));
        scope.while_visible(move || {
            effect(move || {
                let next = report
                    .data()
                    .map_or_else(Vec::new, |account| rows_of(&account.positions));
                rows.set(Shared::new(next));
            });
        });
        Self {
            portfolio,
            report,
            rows,
        }
    }
}
