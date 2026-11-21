//! The DeFi Protocols' session ([`DefiProtocolsSession`]): the protocol
//! list its settings and its search ask for, and the table its view shows
//! of it (the widget's data, described in the parent module).

use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    rc::Rc,
    time::Duration,
};

use nsh_core::prelude::*;

use super::settings::{Column, DefiProtocolsSettings};
use crate::{
    data::{
        SessionScope,
        ferris::{
            Ferris,
            defillama::{ProtocolFilter, ProtocolRow, Protocols},
        },
        net::{ApiError, QueryState, Watch},
    },
    format,
    widgets::common::Shared,
};

/// The most protocols listed: what Ferris sends unless asked for more. The
/// strip under the table says how many more match.
pub(super) const LIMIT: u32 = 500;
/// How long the search waits after a keystroke before it asks.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(250);
/// What a cell with nothing to show reads.
pub(super) const NONE: &str = "—";

/// How the list stands: what the widget says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// Asked for, not answered yet.
    Loading,
    /// A search asked, not answered yet.
    Searching,
    /// As Ferris last got it from DefiLlama.
    Ready,
    /// Ferris's last good copy: DefiLlama did not answer it.
    Stale,
    /// Shown as it last came: the latest request failed, with this.
    Behind(String),
    /// Nothing to show: the request failed, with this.
    Failed(String),
}

impl Standing {
    /// How a request with `data` and its latest `error` stands.
    pub fn of(data: Option<&Protocols>, error: Option<&ApiError>) -> Self {
        match (data, error) {
            (None, None) => Self::Loading,
            (None, Some(error)) => Self::Failed(error.message.clone()),
            (Some(_), Some(error)) => Self::Behind(error.message.clone()),
            (Some(list), None) if list.stale => Self::Stale,
            (Some(_), None) => Self::Ready,
        }
    }

    /// What the strip over the table says in plain text; nothing once
    /// there is an answer.
    pub fn note(&self) -> Option<&'static str> {
        match self {
            Self::Loading => Some("Loading protocols…"),
            Self::Searching => Some("Searching…"),
            _ => None,
        }
    }

    /// What the strip over the table warns of; nothing while all is well.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Loading | Self::Searching | Self::Ready => None,
            Self::Stale => Some(
                "DefiLlama is not answering: this is the last list Lattice's backend has."
                    .to_owned(),
            ),
            Self::Behind(error) => Some(format!(
                "Could not refresh the list ({error}): showing it as it last came."
            )),
            Self::Failed(error) => Some(format!("Could not load the list: {error}")),
        }
    }
}

/// What colors a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Plain,
    Muted,
    Faint,
    Up,
    Down,
}

/// A figure in percent as the table writes it, toned by what shows of it:
/// one that rounds to nothing has no color.
fn change(percent: Option<f64>) -> (String, Tone) {
    let Some(percent) = percent else {
        return (NONE.to_owned(), Tone::Faint);
    };
    let text = format::signed_percent(percent / 100.0);
    let tone = match text.as_bytes().first() {
        Some(b'+') => Tone::Up,
        Some(b'-') => Tone::Down,
        _ => Tone::Muted,
    };
    (text, tone)
}

/// A figure in US dollars as the table writes it.
fn dollars(usd: Option<f64>) -> (String, Tone) {
    match usd {
        Some(usd) => (format::compact_usd(usd), Tone::Plain),
        None => (NONE.to_owned(), Tone::Faint),
    }
}

/// What `row`'s cell of `column` reads, `rank` being its place by TVL.
pub(super) fn cell_of(rank: usize, row: &ProtocolRow, column: Column) -> (String, Tone) {
    match column {
        Column::Rank => (rank.to_string(), Tone::Faint),
        Column::Name => (row.name.clone(), Tone::Plain),
        Column::Tvl => dollars(row.tvl),
        Column::Change1h => change(row.change_1h),
        Column::Change1d => change(row.change_1d),
        Column::Change7d => change(row.change_7d),
        Column::Category => match &row.category {
            Some(category) => (category.clone(), Tone::Muted),
            None => (NONE.to_owned(), Tone::Faint),
        },
        Column::Chains => match row.chains.as_slice() {
            [] => (NONE.to_owned(), Tone::Faint),
            [only] => (only.clone(), Tone::Muted),
            [first, more @ ..] => (format!("{first} +{}", more.len()), Tone::Muted),
        },
        Column::Mcap => dollars(row.mcap),
        Column::McapTvl => match row.mcap_to_tvl() {
            Some(ratio) => (format!("{ratio:.2}"), Tone::Plain),
            None => (NONE.to_owned(), Tone::Faint),
        },
    }
}

/// `row`'s figure in `column`, for a column ordered by one.
fn figure_of(row: &ProtocolRow, column: Column) -> Option<f64> {
    match column {
        Column::Tvl => row.tvl,
        Column::Change1h => row.change_1h,
        Column::Change1d => row.change_1d,
        Column::Change7d => row.change_7d,
        Column::Chains => (!row.chains.is_empty()).then_some(row.chains.len() as f64),
        Column::Mcap => row.mcap,
        Column::McapTvl => row.mcap_to_tvl(),
        Column::Rank | Column::Name | Column::Category => None,
    }
}

/// `list`'s rows in the order `column` and its direction give them, as
/// places in the list (which is by TVL, so a place is a rank less one).
/// A row with nothing in the column comes last either way, and rows that
/// tie keep their order by TVL.
pub(super) fn order_of(list: &Protocols, column: Column, descending: bool) -> Vec<usize> {
    let rows = &list.protocols;
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let directed = |ordering: Ordering| {
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    };
    match column {
        // The list's own order, largest first.
        Column::Rank => {}
        Column::Tvl if descending => {}
        Column::Name => {
            order.sort_by(|a, b| directed(format::compare_text(&rows[*a].name, &rows[*b].name)));
        }
        Column::Category => order.sort_by(|a, b| {
            match (rows[*a].category.as_deref(), rows[*b].category.as_deref()) {
                (Some(a), Some(b)) => directed(format::compare_text(a, b)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }),
        _ => order.sort_by(|a, b| {
            match (figure_of(&rows[*a], column), figure_of(&rows[*b], column)) {
                (Some(a), Some(b)) => directed(a.total_cmp(&b)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }),
    }
    order
}

/// What the table shows: a list, and its rows in order.
#[derive(Default)]
pub(super) struct Table {
    /// The list shown; none until one came.
    pub list: Option<Rc<Protocols>>,
    /// Places in the list, in the order shown.
    order: Vec<usize>,
}

impl Table {
    /// `list` ordered by `column`.
    pub fn of(list: Option<Rc<Protocols>>, column: Column, descending: bool) -> Self {
        let order = list
            .as_deref()
            .map_or_else(Vec::new, |list| order_of(list, column, descending));
        Self { list, order }
    }

    /// How many rows it shows.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// The row shown at `at`, with its rank by TVL.
    pub fn row(&self, at: usize) -> Option<(usize, &ProtocolRow)> {
        let place = *self.order.get(at)?;
        let row = self.list.as_ref()?.protocols.get(place)?;
        Some((place + 1, row))
    }
}

/// What the search has to show.
#[derive(Clone, PartialEq)]
enum Searched {
    /// Nothing typed: the list is browsed.
    Off,
    /// Asked, with nothing found before it: the list browsed shows
    /// meanwhile.
    Asking,
    Got(Shared<Protocols>),
    Failed(String),
}

/// A DeFi Protocols widget's session state: what is typed in its search,
/// and what its view shows (its watch lives in the effect `new` starts).
pub(crate) struct DefiProtocolsSession {
    /// What is typed in the search field, kept while the widget is in the
    /// workspace.
    pub(super) search: Signal<String>,
    /// The rows, in order, made again while the widget shows.
    pub(super) table: Signal<Shared<Table>>,
    /// How the list stands.
    pub(super) standing: Memo<Standing>,
    /// Every category seen in a list so far, by name: what the category
    /// select offers.
    pub(super) categories: Signal<Vec<String>>,
}

impl DefiProtocolsSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<DefiProtocolsSettings>,
    ) -> Self {
        let search = signal(String::new());
        let category = computed(move || settings.with(|s| s.category().map(str::to_owned)));
        let sorting = computed(move || settings.with(|s| (s.sort(), s.descending())));

        // The list browsed: one watch per category, shared by every widget
        // listing it and asked again while watched. Another category is
        // watched before the last one is let go, under the session.
        let feed = signal(None::<ReadSignal<QueryState<Protocols>>>);
        {
            let ferris = ferris.clone();
            let held: RefCell<Option<Watch<Protocols>>> = RefCell::new(None);
            effect(move || {
                let filter = ProtocolFilter {
                    category: category.get(),
                    limit: Some(LIMIT),
                    ..ProtocolFilter::default()
                };
                untracked(|| {
                    let watch = scope.run(|| ferris.watch_defillama_protocols(&filter));
                    feed.set(Some(watch.state()));
                    *held.borrow_mut() = Some(watch);
                });
            });
        }

        // The search: asked once typing has stopped, one request per text
        // (shared and kept by the query hub, so the same text again asks
        // nothing), and shown unless something else was typed meanwhile.
        // What was found last stays until the next answer.
        let searched = signal(Searched::Off);
        {
            let ferris = ferris.clone();
            let asked = Rc::new(Cell::new(0_u64));
            effect(move || {
                let wanted = search.with(|text| text.trim().to_owned());
                let category = category.get();
                let turn = asked.get().wrapping_add(1);
                asked.set(turn);
                if wanted.is_empty() {
                    searched.try_set(Searched::Off);
                    return;
                }
                if searched.with_untracked(|searched| *searched == Searched::Off) {
                    searched.try_set(Searched::Asking);
                }
                let (ferris, asked) = (ferris.clone(), asked.clone());
                spawn(async move {
                    timer(DEBOUNCE).await;
                    if asked.get() != turn {
                        return;
                    }
                    let filter = ProtocolFilter {
                        category,
                        search: Some(wanted),
                        limit: Some(LIMIT),
                        ..ProtocolFilter::default()
                    };
                    let query = ferris.defillama_protocols_query(&filter);
                    let answer = ferris.queries().get(query).await;
                    if asked.get() != turn {
                        return;
                    }
                    searched.try_set(match answer {
                        Ok(list) => Searched::Got(Shared(list)),
                        Err(error) => Searched::Failed(error.message),
                    });
                });
            });
        }

        // What is listed, and how it stands: the search's answer while
        // something is typed, else the list browsed.
        let listing = computed(move || match searched.get() {
            // The first search's answer is on its way: the list browsed
            // stays until it comes.
            Searched::Asking => (
                feed.get()
                    .and_then(|state| state.with(|state| state.data.clone()))
                    .map(Shared),
                Standing::Searching,
            ),
            Searched::Got(list) => {
                let standing = Standing::of(Some(&list.0), None);
                (Some(list), standing)
            }
            Searched::Failed(error) => (None, Standing::Failed(error)),
            Searched::Off => match feed.get() {
                Some(state) => state.with(|state| {
                    (
                        state.data.clone().map(Shared),
                        Standing::of(state.data.as_deref(), state.error.as_ref()),
                    )
                }),
                None => (None, Standing::Loading),
            },
        });
        let standing = computed(move || listing.with(|(_, standing)| standing.clone()));

        // The categories: every one a list has named, so the select offers
        // them whichever is listed now. The whole list (no category) is
        // asked once for them: the request the widget browses by default.
        let categories = signal(Vec::<String>::new());
        let learn = move |list: &Protocols| {
            let mut known = categories.get_untracked();
            let before = known.len();
            for category in list
                .protocols
                .iter()
                .filter_map(|row| row.category.as_ref())
            {
                if !known.contains(category) {
                    known.push(category.clone());
                }
            }
            if known.len() != before {
                known.sort_by(|a, b| format::compare_text(a, b));
                categories.try_set(known);
            }
        };
        effect(move || {
            if let Some(list) = listing.with(|(list, _)| list.clone()) {
                untracked(|| learn(&list.0));
            }
        });
        {
            let ferris = ferris.clone();
            spawn(async move {
                let every = ProtocolFilter {
                    limit: Some(LIMIT),
                    ..ProtocolFilter::default()
                };
                let query = ferris.defillama_protocols_query(&every);
                if let Ok(list) = ferris.queries().get(query).await {
                    learn(&list);
                }
            });
        }

        let table = signal(Shared::new(Table::default()));
        scope.while_visible(move || {
            effect(move || {
                let list = listing.with(|(list, _)| list.as_ref().map(|list| list.0.clone()));
                let (column, descending) = sorting.get();
                table.set(Shared::new(Table::of(list, column, descending)));
            });
        });

        Self {
            search,
            table,
            standing,
            categories,
        }
    }
}
