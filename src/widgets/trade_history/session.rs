//! The Trade History's session ([`TradeHistorySession`]): the trades of
//! each account its settings name, and the rows and totals its view shows
//! of them together (the widget's data, described in the parent module).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::settings::TradeHistorySettings;
use crate::{
    data::{
        Portfolio, SessionScope,
        portfolio::{Fill, Fills, TradeSide},
    },
    format,
    widgets::common::{
        Shared,
        accounts::{AccountSettings, Mark, NONE, Reports, Tone, follow_all, shown},
    },
};

/// The most trades the table lists: the latest. A row is a view per cell,
/// and what is further back is the exchange's own history to read.
pub(super) const MAX_ROWS: usize = 200;

/// The asset fees are written in dollars for.
const QUOTE: &str = "USDC";

/// One trade's row, as its cells show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    /// What tells the row apart: the trade's id, after its account when
    /// the widget shows several.
    pub key: Rc<str>,
    /// Whose it is, when the widget shows several accounts.
    pub owner: Option<Rc<Mark>>,
    /// When, in Unix milliseconds (the view writes it as the clock does).
    pub time_ms: f64,
    pub instrument: String,
    pub side: TradeSide,
    /// What it did (`Close Long`), or its side when not reported.
    pub action: String,
    /// `Taker` or `Maker`; empty when not reported.
    pub liquidity: &'static str,
    pub price: String,
    pub size: String,
    pub value: String,
    pub fee: String,
    /// What it closed, a dash when it closed nothing.
    pub pnl: String,
    pub tone: Option<Tone>,
}

/// A fee: dollars for the quote currency (`$0.228173`, `-$0.01` for a
/// rebate), its asset's units for another (`0.0012 HYPE`).
fn fee_text(fill: &Fill) -> String {
    let Some(fee) = fill.fee else {
        return NONE.to_owned();
    };
    match fill.fee_asset.as_deref() {
        Some(asset) if asset != QUOTE => format!("{} {asset}", format::quantity(fee)),
        _ => {
            let magnitude = format::quantity(fee.abs());
            if fee < 0.0 && magnitude != "0" {
                format!("-${magnitude}")
            } else {
                format!("${magnitude}")
            }
        }
    }
}

impl Row {
    fn of(fill: &Fill, owner: Option<&Rc<Mark>>) -> Self {
        Self {
            key: Mark::key(owner, &fill.id),
            owner: owner.cloned(),
            time_ms: fill.time_ms,
            instrument: fill.instrument.clone(),
            side: fill.side,
            action: fill
                .action
                .clone()
                .unwrap_or_else(|| fill.side.label().to_owned()),
            liquidity: match fill.taker {
                Some(true) => "Taker",
                Some(false) => "Maker",
                None => "",
            },
            price: format::price_text(fill.price),
            size: format::quantity(fill.size),
            value: format::usd(fill.value()),
            fee: fee_text(fill),
            pnl: shown(fill.closed_pnl, format::signed_usd),
            tone: fill.closed_pnl.map(Tone::of),
        }
    }
}

/// What the trades reported come to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Totals {
    /// How many trades were reported, and how many the table lists.
    pub count: usize,
    pub listed: usize,
    /// When the oldest was, in Unix milliseconds.
    pub since_ms: Option<f64>,
    pub volume: String,
    /// The fees paid in the quote currency (less rebates).
    pub fees: String,
    pub closed_pnl: String,
    pub tone: Option<Tone>,
}

impl Totals {
    /// The totals of one account's `fills`; dashes before a report.
    #[cfg(test)]
    pub fn of(fills: Option<&Fills>) -> Self {
        Self::of_all(fills.map(|fills| vec![fills]).as_deref())
    }

    /// The totals of the trades `parts` hold, the accounts' together;
    /// dashes before a report.
    pub fn of_all(parts: Option<&[&Fills]>) -> Self {
        let Some(parts) = parts else {
            return Self {
                count: 0,
                listed: 0,
                since_ms: None,
                volume: NONE.to_owned(),
                fees: NONE.to_owned(),
                closed_pnl: NONE.to_owned(),
                tone: None,
            };
        };
        let fills: Vec<&Fill> = parts.iter().flat_map(|Fills(fills)| fills).collect();
        let volume: f64 = fills.iter().map(|fill| fill.value()).sum();
        let fees: f64 = fills
            .iter()
            .filter(|fill| fill.fee_asset.as_deref().is_none_or(|asset| asset == QUOTE))
            .filter_map(|fill| fill.fee)
            .sum();
        let closed: f64 = fills.iter().filter_map(|fill| fill.closed_pnl).sum();
        Self {
            count: fills.len(),
            listed: fills.len().min(MAX_ROWS),
            since_ms: fills.iter().map(|fill| fill.time_ms).min_by(f64::total_cmp),
            volume: format::usd(volume),
            fees: format::usd(fees),
            closed_pnl: format::signed_usd(closed),
            tone: Some(Tone::of(closed)),
        }
    }
}

/// The rows of one account's `fills`, in the table's order.
#[cfg(test)]
pub(super) fn rows_of(fills: &Fills) -> Vec<Row> {
    rows_of_all(&[(None, fills)])
}

/// The rows of the trades `parts` hold, each account's with its mark (none
/// when the widget shows one account), newest first whichever account each
/// is of, a trade once: the latest [`MAX_ROWS`].
pub(super) fn rows_of_all(parts: &[(Option<Rc<Mark>>, &Fills)]) -> Vec<Row> {
    let mut ordered: Vec<(Option<&Rc<Mark>>, &Fill)> = parts
        .iter()
        .flat_map(|(mark, fills)| fills.0.iter().map(move |fill| (mark.as_ref(), fill)))
        .collect();
    ordered.sort_by(|(_, a), (_, b)| b.time_ms.total_cmp(&a.time_ms));
    let mut rows: Vec<Row> = Vec::with_capacity(ordered.len().min(MAX_ROWS));
    for (mark, fill) in ordered {
        if rows.len() == MAX_ROWS {
            break;
        }
        let row = Row::of(fill, mark);
        if !rows.iter().any(|kept| kept.key == row.key) {
            rows.push(row);
        }
    }
    rows
}

/// A Trade History's session state: its accounts' trades, and what its
/// view shows of them (its watches live in the effect `follow_all`
/// starts).
pub(crate) struct TradeHistorySession {
    /// The exchanges that report accounts, which its account's rows list.
    pub(super) portfolio: Portfolio,
    /// The trades of each account the settings name, and where they stand
    /// as one.
    pub(super) reports: Reports<Fills>,
    /// The rows, newest first, updated while the widget is visible.
    pub(super) rows: Signal<Shared<Vec<Row>>>,
    /// What the trades come to, updated while it is visible.
    pub(super) totals: Signal<Totals>,
}

impl TradeHistorySession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        portfolio: Portfolio,
        settings: Signal<TradeHistorySettings>,
    ) -> Self {
        let accounts = computed(move || settings.with(|settings| settings.accounts().to_vec()));
        let reports = {
            let portfolio = portfolio.clone();
            follow_all(scope, accounts, move |account| {
                portfolio.watch_fills(account)
            })
        };
        let rows = signal(Shared::new(Vec::new()));
        let totals = signal(Totals::of_all(None));
        {
            let portfolio = portfolio.clone();
            scope.while_visible(move || {
                let portfolio = portfolio.clone();
                effect(move || {
                    // A trade says whose it is once there are several
                    // accounts to be of.
                    let several = accounts.with(|accounts| accounts.len() > 1);
                    let arrived = reports.arrived();
                    let parts: Vec<(Option<Rc<Mark>>, &Fills)> = arrived
                        .iter()
                        .flatten()
                        .map(|(account, fills)| {
                            (Mark::when(several, &portfolio, account), &**fills)
                        })
                        .collect();
                    let all: Option<Vec<&Fills>> = arrived
                        .as_ref()
                        .map(|arrived| arrived.iter().map(|(_, fills)| &**fills).collect());
                    rows.set(Shared::new(rows_of_all(&parts)));
                    totals.set(Totals::of_all(all.as_deref()));
                });
            });
        }
        Self {
            portfolio,
            reports,
            rows,
            totals,
        }
    }
}
