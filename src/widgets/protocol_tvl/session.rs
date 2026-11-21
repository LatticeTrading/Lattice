//! The Protocol TVL's session ([`ProtocolTvlSession`]): the TVL of the
//! protocol its settings name, the series it plots of it, its plot, and the
//! search its picker asks DefiLlama's list with (the widget's data,
//! described in the parent module).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use nsh_core::prelude::*;

use super::settings::{ProtocolTvlSettings, Range, Style};
use crate::{
    data::{
        SessionScope,
        ferris::{
            Candle, Ferris,
            defillama::{self, ProtocolFilter, ProtocolTvl, Protocols, TvlPoint},
        },
        net::{ApiError, QueryState, Watch},
    },
    format,
    widgets::common::{
        Shared,
        plot::{Mode, Plot, Viewport},
    },
};

/// The slots a plot with nothing to draw yet looks at.
const UNFITTED: f64 = 100.0;
/// A day, in milliseconds.
const DAY_MS: f64 = 86_400_000.0;
/// How long the search waits after a keystroke before it asks.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(250);
/// The most protocols a search lists.
const SEARCH_LIMIT: u32 = 20;
/// The chains the strip under the plot names before it counts the rest.
const CHAINS_NAMED: usize = 3;

/// What a figure with nothing to show reads.
pub(super) const NONE: &str = "—";
/// What the plot says in its middle while it has no point.
pub(super) const LOADING: &str = "Loading TVL history...";
pub(super) const UNAVAILABLE: &str = "TVL history is unavailable.";
pub(super) const UNLISTED: &str = "DefiLlama does not list this protocol.";
pub(super) const EMPTY: &str = "DefiLlama reports no TVL for this protocol.";

/// How the protocol's TVL stands: what the widget says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// Asked for, not answered yet.
    Loading,
    /// As Ferris last got it from DefiLlama.
    Ready,
    /// Ferris's last good copy: DefiLlama did not answer it.
    Stale,
    /// Shown as it last came: the latest request failed, with this.
    Behind(String),
    /// Nothing to show: the request failed, with this.
    Failed(String),
    /// DefiLlama lists no protocol under the slug.
    Unlisted,
}

impl Standing {
    /// How a request with `data` and its latest `error` stands.
    pub fn of(data: Option<&ProtocolTvl>, error: Option<&ApiError>) -> Self {
        match (data, error) {
            (None, None) => Self::Loading,
            (None, Some(error)) if error.code == defillama::NOT_FOUND => Self::Unlisted,
            (None, Some(error)) => Self::Failed(error.message.clone()),
            (Some(_), Some(error)) => Self::Behind(error.message.clone()),
            (Some(tvl), None) if tvl.stale => Self::Stale,
            (Some(_), None) => Self::Ready,
        }
    }

    /// What the plot says in its middle with no point to plot; nothing
    /// once it has one.
    pub fn said(&self, plotted: bool) -> &'static str {
        match self {
            _ if plotted => "",
            Self::Loading => LOADING,
            Self::Failed(_) => UNAVAILABLE,
            Self::Unlisted => UNLISTED,
            Self::Ready | Self::Stale | Self::Behind(_) => EMPTY,
        }
    }

    /// What the strip over the plot warns of; nothing while all is well.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Loading | Self::Ready | Self::Unlisted => None,
            Self::Stale => Some(
                "DefiLlama is not answering: this is the last TVL Lattice's backend has."
                    .to_owned(),
            ),
            Self::Behind(error) => Some(format!(
                "Could not refresh the TVL ({error}): showing it as it last came."
            )),
            Self::Failed(error) => Some(format!("Could not load the TVL: {error}")),
        }
    }
}

/// What `range` plots of `history`: a bar of one value a point, oldest
/// first, counted back from the latest point (so a history that stopped
/// still fills its range), whole seconds apart at least.
pub(super) fn bars_of(history: &[TvlPoint], range: Range) -> Vec<Candle> {
    let Some(latest) = history.last() else {
        return Vec::new();
    };
    let from = range
        .days()
        .map_or(f64::NEG_INFINITY, |days| latest.time_ms - days * DAY_MS);
    let mut bars: Vec<Candle> = Vec::new();
    for point in history.iter().filter(|point| point.time_ms >= from) {
        let bar = Candle {
            time: (point.time_ms / 1000.0).floor(),
            open: point.usd,
            high: point.usd,
            low: point.usd,
            close: point.usd,
            volume: None,
        };
        match bars.last_mut() {
            Some(last) if last.time >= bar.time => {
                *last = Candle {
                    time: last.time,
                    ..bar
                };
            }
            _ => bars.push(bar),
        }
    }
    bars
}

/// How `style` draws the series: green above its baseline, red below.
pub(super) fn mode_of(style: Style) -> Mode {
    match style {
        Style::Line => Mode::Signed,
        Style::Area => Mode::SignedArea,
    }
}

/// Which way a change reads: what colors it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sign {
    Up,
    Down,
    Flat,
}

/// What the widget says of the range plotted, under its header.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Readout {
    /// What the figure is called: `TVL`, or `Assets` for a centralized
    /// exchange, whose figure is what its known wallets hold.
    pub measure: &'static str,
    /// The latest figure (`$7.151B`), the series' last point.
    pub value: String,
    /// How far it moved over the range plotted (`+12.40%`).
    pub change: String,
    /// What colors [`Self::change`], by what shows of it.
    pub sign: Sign,
}

impl Readout {
    /// The readout of `bars`, the series plotted of `tvl`; dashes with
    /// nothing plotted, and no change from a range that began at nothing.
    pub fn of(tvl: Option<&ProtocolTvl>, bars: &[Candle]) -> Self {
        let measure = match tvl {
            Some(tvl) if tvl.protocol.is_cex() => "Assets",
            _ => "TVL",
        };
        let latest = bars.last().map(|bar| bar.close);
        let moved = match (bars.first(), bars.last()) {
            (Some(first), Some(last)) if bars.len() > 1 && first.close > 0.0 => {
                Some((last.close - first.close) / first.close)
            }
            _ => None,
        };
        let change = moved.map_or_else(|| NONE.to_owned(), format::signed_percent);
        let sign = match change.as_bytes().first() {
            Some(b'+') => Sign::Up,
            Some(b'-') => Sign::Down,
            _ => Sign::Flat,
        };
        Self {
            measure,
            value: latest.map_or_else(|| NONE.to_owned(), format::compact_usd),
            change,
            sign,
        }
    }
}

/// A share of `total` as a percentage with one decimal.
fn percent(usd: f64, total: f64) -> String {
    format!("{:.1}%", usd / total * 100.0)
}

/// The chains `tvl` is on in one line, largest first, each with its share
/// of the total: `Hyperliquid L1 98.9% · Arbitrum 1.1%`, the ones past
/// the third counted (`+6 more`). Nothing for a protocol with no chain.
pub(super) fn chains_line(tvl: &ProtocolTvl) -> String {
    let total: f64 = tvl.chains.iter().map(|chain| chain.usd.max(0.0)).sum();
    if total <= 0.0 {
        return String::new();
    }
    let mut parts: Vec<String> = tvl
        .chains
        .iter()
        .take(CHAINS_NAMED)
        .map(|chain| format!("{} {}", chain.name, percent(chain.usd, total)))
        .collect();
    if tvl.chains.len() > CHAINS_NAMED {
        parts.push(format!("+{} more", tvl.chains.len() - CHAINS_NAMED));
    }
    parts.join(" · ")
}

/// Every chain of `tvl` with what is on it, a line each, then what
/// DefiLlama counts apart from the total: the strip's tooltip.
pub(super) fn chains_detail(tvl: &ProtocolTvl) -> String {
    let mut lines: Vec<String> = tvl
        .chains
        .iter()
        .map(|chain| format!("{} {}", chain.name, format::compact_usd(chain.usd)))
        .collect();
    if !tvl.extras.is_empty() {
        lines.push("Counted apart from the total:".to_owned());
        lines.extend(
            tvl.extras
                .iter()
                .map(|extra| format!("{} {}", extra.name, format::compact_usd(extra.usd))),
        );
    }
    lines.join("\n")
}

/// A slug as a name, for a protocol not asked about yet: its words
/// capitalized (`apex-protocol` reads `Apex Protocol`).
pub(super) fn titled(slug: &str) -> String {
    slug.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut letters = word.chars();
            match letters.next() {
                Some(first) => first.to_uppercase().chain(letters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One protocol the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Choice {
    /// What picking it charts.
    pub slug: String,
    pub name: String,
    /// What it is, beside its name: its category, or that it is every
    /// product of a protocol together.
    pub note: String,
    /// Its TVL, when its list says.
    pub tvl: Option<f64>,
    /// The exchange Lattice shows it as, whose logo leads its row.
    pub exchange: Option<&'static str>,
}

/// What the picker offers before anything is typed: the exchanges Lattice
/// shows that DefiLlama lists, each as the protocol its venue names.
pub(super) fn venue_choices() -> Vec<Choice> {
    defillama::venues()
        .map(|(venue, slug)| Choice {
            slug: slug.to_owned(),
            name: venue.name.to_owned(),
            note: String::new(),
            tvl: None,
            exchange: Some(venue.id),
        })
        .collect()
}

/// What `found` offers: its rows as they come, largest first, and before
/// the first product of each parent protocol the parent itself, which is
/// no row of DefiLlama's list (a parent's total is its own request: what
/// its products share is counted once).
pub(super) fn choices_of(found: &Protocols) -> Vec<Choice> {
    let mut choices: Vec<Choice> = Vec::new();
    let mut parents: Vec<&str> = Vec::new();
    for row in &found.protocols {
        if let Some(parent) = row.parent_slug.as_deref() {
            let listed = found.protocols.iter().any(|row| row.slug == parent);
            if !listed && !parents.contains(&parent) {
                parents.push(parent);
                choices.push(Choice {
                    slug: parent.to_owned(),
                    name: titled(parent),
                    note: "All products".to_owned(),
                    tvl: None,
                    exchange: None,
                });
            }
        }
        choices.push(Choice {
            slug: row.slug.clone(),
            name: row.name.clone(),
            note: row.category.clone().unwrap_or_default(),
            tvl: row.tvl,
            exchange: None,
        });
    }
    choices
}

/// What the picker lists.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Found {
    /// Nothing typed: the exchanges ([`venue_choices`]).
    Venues,
    /// Asked, not answered yet.
    Searching,
    Listed(Vec<Choice>),
    Failed(String),
}

/// The picker's search, kept by the session so it is as it was left when
/// the widget's page shows again.
#[derive(Clone, Copy)]
pub(super) struct Search {
    /// Whether the picker is open: nothing is asked while it is not.
    pub open: Signal<bool>,
    /// What is typed.
    pub text: Signal<String>,
    /// What it lists for what is typed.
    pub found: Signal<Found>,
}

impl Search {
    /// A search that asks `ferris` for what is typed, once typing has
    /// stopped for [`DEBOUNCE`], and lists the answer unless something
    /// else was typed meanwhile (what it listed before stays until then). One request per text, shared and kept by
    /// the query hub, so typing the same again asks nothing.
    fn new(ferris: Ferris) -> Self {
        let (open, text) = (signal(false), signal(String::new()));
        let found = signal(Found::Venues);
        let asked = Rc::new(Cell::new(0_u64));
        effect(move || {
            let wanted = text.with(|text| text.trim().to_owned());
            let turn = asked.get().wrapping_add(1);
            asked.set(turn);
            if !open.get() {
                return;
            }
            if wanted.is_empty() {
                found.try_set(Found::Venues);
                return;
            }
            // What was found last stays until the next answer.
            if !found.with_untracked(|found| matches!(found, Found::Listed(_))) {
                found.try_set(Found::Searching);
            }
            let (ferris, asked) = (ferris.clone(), asked.clone());
            spawn(async move {
                timer(DEBOUNCE).await;
                if asked.get() != turn {
                    return;
                }
                let filter = ProtocolFilter {
                    search: Some(wanted),
                    limit: Some(SEARCH_LIMIT),
                    ..ProtocolFilter::default()
                };
                let query = ferris.defillama_protocols_query(&filter);
                let answer = ferris.queries().get(query).await;
                if asked.get() != turn {
                    return;
                }
                found.try_set(match answer {
                    Ok(list) => Found::Listed(choices_of(&list)),
                    Err(error) => Found::Failed(error.message),
                });
            });
        });
        Self { open, text, found }
    }
}

/// The series the plot last started over for, and how it stood then.
#[derive(Clone, PartialEq)]
struct Fitted {
    /// What was plotted: whose, and which range of it.
    series: (String, Range),
    /// How many points it had.
    points: usize,
    /// How many times the user had moved the viewport.
    moves: u64,
}

/// A Protocol TVL's session state: its protocol's TVL, its plot, and what
/// its view says of the range plotted (its watch lives in the effect `new`
/// starts).
pub(crate) struct ProtocolTvlSession {
    /// The protocol's TVL as it last came, whatever came after.
    pub(super) tvl: Memo<Option<Shared<ProtocolTvl>>>,
    /// How it stands.
    pub(super) standing: Memo<Standing>,
    /// Its plot. Where it looks starts over, on the whole range, when
    /// another series is plotted, and follows the range as it grows until
    /// the user moves it.
    pub(super) plot: Rc<Plot>,
    /// The latest figure and how far it moved, updated while it shows.
    pub(super) readout: Signal<Readout>,
    /// What the plot says in its middle while it has no point.
    pub(super) status: Signal<&'static str>,
    /// The picker's search.
    pub(super) search: Search,
}

impl ProtocolTvlSession {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<ProtocolTvlSettings>,
    ) -> Self {
        let slug = computed(move || settings.with(|settings| settings.protocol().to_owned()));
        // The watch follows the settings: another protocol is watched
        // before the last one is let go, under the session, so it lives as
        // long as the widget is in the workspace.
        let feed = signal(None::<ReadSignal<QueryState<ProtocolTvl>>>);
        {
            let ferris = ferris.clone();
            let held: RefCell<Option<Watch<ProtocolTvl>>> = RefCell::new(None);
            effect(move || {
                let slug = slug.get();
                untracked(|| {
                    let watch = scope.run(|| ferris.watch_defillama_protocol(&slug));
                    feed.set(Some(watch.state()));
                    *held.borrow_mut() = Some(watch);
                });
            });
        }
        let tvl = computed(move || {
            feed.get()
                .and_then(|state| state.with(|state| state.data.clone()))
                .map(Shared)
        });
        let standing = computed(move || match feed.get() {
            Some(state) => {
                state.with(|state| Standing::of(state.data.as_deref(), state.error.as_ref()))
            }
            None => Standing::Loading,
        });
        let picked =
            computed(move || settings.with(|settings| (settings.range(), settings.style())));

        let plot = Plot::new(Mode::Signed, UNFITTED);
        let readout = signal(Readout::of(None, &[]));
        let status = signal(LOADING);
        let fitted: Rc<RefCell<Option<Fitted>>> = Rc::default();
        {
            let plot = plot.clone();
            scope.while_visible(move || {
                // New data, another protocol or another range: the plot
                // draws again.
                {
                    let plot = plot.clone();
                    effect(move || {
                        tvl.with(|_| ());
                        standing.with(|_| ());
                        slug.with(|_| ());
                        picked.get();
                        plot.mark();
                    });
                }
                let (drawn, fitted) = (plot.clone(), fitted.clone());
                plot.drive(move || {
                    let plot = &drawn;
                    let (range, style) = picked.get_untracked();
                    let (tvl, standing) = untracked(|| (tvl.get(), standing.get()));
                    let tvl = tvl.as_ref().map(|tvl| &*tvl.0);
                    let bars = tvl.map_or_else(Vec::new, |tvl| bars_of(&tvl.history, range));
                    // Another series starts over on the whole of it; the
                    // same one grown is followed until the user looks
                    // elsewhere.
                    let series = (slug.get_untracked(), range);
                    let refit = match &*fitted.borrow() {
                        None => true,
                        Some(last) => {
                            last.series != series
                                || (last.points != bars.len() && last.moves == plot.moves())
                        }
                    };
                    if refit {
                        *fitted.borrow_mut() = Some(Fitted {
                            series,
                            points: bars.len(),
                            moves: plot.moves(),
                        });
                        plot.viewport
                            .set(Viewport::showing(bars.len().max(2) as f64));
                    }
                    let (held, scene) = plot.frame(&bars, 0, mode_of(style));
                    // Green above what the range began at, red below.
                    let scene = scene.with_baseline(bars.first().map_or(0.0, |first| first.close));
                    batch(|| {
                        plot.show(held, scene);
                        readout.try_set(Readout::of(tvl, &bars));
                        status.try_set(standing.said(!bars.is_empty()));
                    });
                });
            });
        }

        Self {
            tvl,
            standing,
            plot,
            readout,
            status,
            search: Search::new(ferris),
        }
    }
}
