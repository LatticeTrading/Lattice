//! The Account Summary's view: the frame every account widget has (the
//! accounts' strip, what could not be read, the form while no account is
//! set: [`account_frame`]) around three parts that scroll together:
//!
//! - the headline: what the whole account is worth and what it made in the
//!   last day, over a bar of what it is made of (perpetuals, spot, staked,
//!   vaults, and what else the exchange counts), each part named under it
//!   with its worth and share;
//! - the tiles, in a grid as many across as the pane's width affords (one
//!   below [`TILE_MIN`] each, so a narrow pane is a list): Margin Health
//!   (the cross margin ratio over a gauge green, amber and red as it nears
//!   liquidation), Margin (how much of the equity the margin holds),
//!   Exposure (long against short), Profit & Loss, Fees (the tier, over how
//!   far its volume has gone to the next) and Staking & Vaults. Each is a
//!   title with its headline figure, then a row per figure;
//! - Balances, while the settings show them: what is to be said of them
//!   (they are a report of their own), then a row per asset under titles
//!   (each marked with whose it is while several accounts show, an
//!   exchange's logo named in its tooltip): how much, how much of it is free, what it is worth, what it cost and
//!   what it has made, the most valuable first. The columns share the
//!   pane's width, so a narrow pane cuts a figure short rather than
//!   scrolling sideways.
//!
//! Every part is made once and follows the session's overview, so a new
//! report rewrites text and redraws bars and makes no view.

use nsh_core::prelude::*;

use super::{
    session::{AccountSummarySession, Figure, Health, Holding, Overview, Part},
    settings::AccountSummarySettings,
};
use crate::{
    theme::{self, Role},
    ui,
    widgets::{
        common::{
            accounts::{account_frame, mark_view, report_note, Tone},
            Shared,
        },
        TabContext,
    },
};

/// What its test ids start with.
const ID: &str = "account-summary";
/// A balances row's height, and a section's or column's title's, each with
/// the line under it.
const ROW_HEIGHT: f32 = 31.0;
const TITLE_HEIGHT: f32 = 27.0;
/// A tile's row.
const LINE_HEIGHT: f32 = 22.0;
/// The side padding of rows and tiles.
const PADDING: f32 = 12.0;
/// The narrowest a tile is: a pane with no room for two is a list.
const TILE_MIN: f32 = 280.0;
/// A bar's thickness: a tile's gauge, and the headline's parts.
const GAUGE: f32 = 4.0;
const PARTS: f32 = 6.0;
/// The balances' columns, which share the pane's width.
const COLUMNS: &str =
    "minmax(0,1.1fr) minmax(0,1fr) minmax(0,1fr) minmax(0,1fr) minmax(0,1fr) minmax(0,1fr)";

/// A column of the balances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Asset,
    Total,
    Available,
    Value,
    Cost,
    Pnl,
}

impl Column {
    /// Every column, in the table's order.
    const ALL: [Self; 6] = [
        Self::Asset,
        Self::Total,
        Self::Available,
        Self::Value,
        Self::Cost,
        Self::Pnl,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Asset => "Asset",
            Self::Total => "Total",
            Self::Available => "Available",
            Self::Value => "Value",
            Self::Cost => "Cost",
            Self::Pnl => "PnL",
        }
    }

    /// What `holding`'s cell of it reads.
    fn figure(self, holding: &Holding) -> String {
        match self {
            Self::Asset => holding.asset.clone(),
            Self::Total => holding.total.clone(),
            Self::Available => holding.available.clone(),
            Self::Value => holding.value.clone(),
            Self::Cost => holding.cost.clone(),
            Self::Pnl => holding.pnl.clone(),
        }
    }
}

/// What a tile draws under its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gauge {
    /// The cross margin ratio, in its health's color.
    Health,
    /// How much of the equity the margin holds.
    Usage,
    /// Long against short.
    Sides,
    /// How far the volume has gone to the next fee tier.
    Tier,
}

/// One tile of the grid.
struct Tile {
    title: &'static str,
    /// The figure beside the title.
    headline: Option<Figure>,
    gauge: Option<Gauge>,
    rows: &'static [Figure],
}

/// The tiles, in the grid's order.
const TILES: [Tile; 6] = [
    Tile {
        title: "Margin Health",
        headline: Some(Figure::MarginRatio),
        gauge: Some(Gauge::Health),
        rows: &[Figure::Buffer, Figure::Maintenance, Figure::CrossEquity],
    },
    Tile {
        title: "Margin",
        headline: Some(Figure::Equity),
        gauge: Some(Gauge::Usage),
        rows: &[
            Figure::MarginUsed,
            Figure::Withdrawable,
            Figure::Leverage,
            Figure::Isolated,
            Figure::IsolatedMargin,
        ],
    },
    Tile {
        title: "Exposure",
        headline: Some(Figure::Gross),
        gauge: Some(Gauge::Sides),
        rows: &[
            Figure::Long,
            Figure::Short,
            Figure::Net,
            Figure::Positions,
            Figure::Largest,
        ],
    },
    Tile {
        title: "Profit & Loss",
        headline: Some(Figure::Unrealized),
        gauge: None,
        rows: &[
            Figure::Funding,
            Figure::WeekPnl,
            Figure::MonthPnl,
            Figure::DayVolume,
        ],
    },
    Tile {
        title: "Fees",
        headline: Some(Figure::FeeTier),
        gauge: Some(Gauge::Tier),
        rows: &[
            Figure::FeeVolume,
            Figure::NextTier,
            Figure::PerpRates,
            Figure::SpotRates,
            Figure::Discount,
        ],
    },
    Tile {
        title: "Staking & Vaults",
        headline: Some(Figure::StakedValue),
        gauge: None,
        rows: &[
            Figure::Staked,
            Figure::Undelegated,
            Figure::Unstaking,
            Figure::VaultEquity,
            Figure::VaultCount,
        ],
    },
];

/// The color `part` is drawn in, in the bar and beside its name.
fn part_color(part: Part) -> Role {
    match part {
        Part::Perps => theme::LINK_BLUE,
        Part::Spot => theme::LINK_GREEN,
        Part::Staked => theme::LINK_YELLOW,
        Part::Vaults => theme::LINK_ORANGE,
        Part::Other => theme::TEXT_FAINT,
    }
}

/// The color of a cross margin ratio (tracked).
fn health_color(ratio: Option<f64>) -> AdaptiveColor {
    match ratio.map(Health::of) {
        Some(Health::Safe) => theme::POSITIVE.get(),
        Some(Health::Caution) => theme::WARNING.get(),
        Some(Health::Danger) => theme::NEGATIVE.get(),
        None => theme::TEXT.get(),
    }
}

/// The Account Summary of `session`, for the account its tab's settings
/// name.
pub(super) fn summary_view(
    session: &AccountSummarySession,
    tab: &TabContext<AccountSummarySettings>,
) -> View {
    let saved = tab.settings;
    let (overview, holdings) = (session.overview, session.holdings);
    let balances = session.balances.standing;
    let hidden = computed(move || !saved.with(AccountSummarySettings::balances));
    // Nothing held: said once every account has arrived to say it.
    let held = session.balances;
    let none_held =
        computed(move || held.settled() && holdings.with(|holdings| holdings.0.is_empty()));
    let empty = view()
        .test_id(&format!("{ID}-no-balances"))
        .px(px(PADDING))
        .py(px(8.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .style_when(move || !none_held.get(), |s| s.hidden())
        .child(text("No balances"));
    let held = view()
        .test_id(&format!("{ID}-balances"))
        .flex_col()
        .style_when(move || hidden.get(), |s| s.hidden())
        .child(section("Balances"))
        .child(report_note(tab, &session.portfolio, balances, "balances"))
        .child(table(holdings))
        .child(empty);
    // One selection across the whole of it, as a page's: a figure's name
    // and value copy together.
    let content = view()
        .selectable(true)
        .flex_col()
        .child(headline(overview))
        .child(tiles(overview))
        .child(held);
    let body = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .child(scroll().absolute().inset_0().child(content));
    account_frame(
        tab,
        &session.portfolio,
        session.margin.standing,
        "the account",
        body,
    )
}

/// A title in the widget's small capitals.
fn heading(name: &'static str) -> Text {
    text(name)
        .number_of_lines(1)
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.8))
        .uppercase()
        .select_text()
        .text_color(theme::TEXT_FAINT)
}

/// A section's title, across the widget.
fn section(name: &'static str) -> View {
    view()
        .h(px(TITLE_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .px(px(PADDING))
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(heading(name))
}

/// What `figure` reads now (tracked).
fn reading(overview: Signal<Overview>, figure: Figure) -> impl Fn() -> String + Copy + 'static {
    move || overview.with(|overview| overview.figure(figure).text.clone())
}

/// The color `figure` is drawn in now (tracked): its gain or loss, the
/// margin ratio its health.
fn coloring(overview: Signal<Overview>, figure: Figure) -> impl Fn() -> AdaptiveColor + 'static {
    move || {
        overview.with(|overview| match figure {
            Figure::MarginRatio => health_color(overview.margin_ratio),
            _ => Tone::color(overview.figure(figure).tone),
        })
    }
}

/// `figure`'s value, as a row or a headline shows it.
fn value(overview: Signal<Overview>, figure: Figure) -> Text {
    // A text does not shrink unless told: without it, a figure wider than
    // its room runs past its tile instead of being cut short.
    text(reading(overview, figure))
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .truncate()
        .tabular_nums()
        .select_text()
        .text_color(coloring(overview, figure))
}

/// A bar `fraction` full (tracked; `None` is empty) in `color`.
fn bar(
    thickness: f32,
    fraction: impl Fn() -> Option<f64> + Copy + 'static,
    color: impl Fn() -> AdaptiveColor + 'static,
) -> View {
    let full = move || fraction().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    track(thickness)
        .child(view().flex_grow(full).bg(color))
        .child(view().flex_grow(move || 1.0 - full()))
}

/// What a bar is drawn in: a row of its segments over the empty color.
fn track(thickness: f32) -> View {
    view()
        .h(px(thickness))
        .flex_none()
        .flex_row()
        .overflow_hidden()
        .rounded_full()
        .bg(theme::BORDER)
}

/// The headline: what the account is worth and made in a day, and what it
/// is made of.
fn headline(overview: Signal<Overview>) -> View {
    let total = view()
        .test_id(&format!("{ID}-{}", Figure::Total.id()))
        .flex_col()
        .gap(px(2.0))
        .min_w(px(0.0))
        .child(heading("Total Value"))
        .child(
            value(overview, Figure::Total)
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD),
        );
    let change = view()
        .flex_col()
        .items_end()
        .gap(px(2.0))
        .flex_none()
        .child(heading("24H"))
        .child(
            view()
                .flex_row()
                .gap(px(6.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .child(
                    view()
                        .test_id(&format!("{ID}-{}", Figure::DayChange.id()))
                        .child(value(overview, Figure::DayChange)),
                )
                .child(
                    view()
                        .test_id(&format!("{ID}-{}", Figure::DayReturn.id()))
                        .child(value(overview, Figure::DayReturn)),
                ),
        );
    let share = move |part: Part| {
        move || overview.with(|overview| overview.share(part).map_or(0.0, |share| share.fraction))
    };
    let parts = track(PARTS)
        .test_id(&format!("{ID}-parts"))
        .children(Part::ALL.map(|part| {
            view()
                .test_id(&format!("{ID}-segment-{}", part.id()))
                .flex_grow(move || share(part)() as f32)
                .bg(part_color(part))
        }));
    let legend = view()
        .flex_row()
        .flex_wrap()
        .gap(px(12.0))
        .children(Part::ALL.map(|part| legend_item(overview, part, share(part))));
    view()
        .test_id(&format!("{ID}-headline"))
        .flex_none()
        .flex_col()
        .gap(px(10.0))
        .px(px(PADDING))
        .py(px(12.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            view()
                .flex_row()
                .items_end()
                .justify_between()
                .gap(px(12.0))
                .child(total)
                .child(change),
        )
        .child(parts)
        .child(legend)
}

/// `part`'s name under the bar, with its worth and share; hidden while it
/// has none.
fn legend_item(
    overview: Signal<Overview>,
    part: Part,
    share: impl Fn() -> f64 + Copy + 'static,
) -> View {
    let of = move |read: fn(&super::session::Share) -> String| {
        move || overview.with(|overview| overview.share(part).map(read).unwrap_or_default())
    };
    view()
        .test_id(&format!("{ID}-part-{}", part.id()))
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .style_when(move || share() <= 0.0, |s| s.hidden())
        .child(
            view()
                .size(px(7.0))
                .flex_none()
                .rounded_full()
                .bg(part_color(part)),
        )
        .child(
            text(part.label())
                .select_text()
                .text_color(theme::TEXT_MUTED),
        )
        .child(
            text(of(|share| share.value.clone()))
                .tabular_nums()
                .font_weight(FontWeight::MEDIUM),
        )
        .child(
            text(of(|share| share.percent.clone()))
                .tabular_nums()
                .text_color(theme::TEXT_FAINT),
        )
}

/// The tiles, as many across as the width affords. Each draws its lines at
/// its right and bottom; the grid runs a line past the pane's right edge so
/// the last column's is not drawn.
fn tiles(overview: Signal<Overview>) -> View {
    let grid = view()
        .test_id(&format!("{ID}-tiles"))
        .grid()
        .grid_template_columns(format!("repeat(auto-fill, minmax({TILE_MIN}px, 1fr))"))
        .mr(px(-1.0))
        .children(
            TILES
                .iter()
                .map(|tile| tile_view(overview, tile))
                .collect::<Vec<_>>(),
        );
    view().flex_none().overflow_hidden().child(grid)
}

/// One tile: its title and headline, its gauge, then its rows.
fn tile_view(overview: Signal<Overview>, tile: &Tile) -> View {
    let title = view()
        .h(px(LINE_HEIGHT))
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(heading(tile.title).flex_none());
    let title = match tile.headline {
        Some(figure) => title.child(
            view()
                .test_id(&format!("{ID}-{}", figure.id()))
                .tooltip(ui::hint(move || figure.label().to_owned()))
                .min_w(px(0.0))
                .flex_shrink(1.0)
                .flex_row()
                .justify_end()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value(overview, figure)),
        ),
        None => title,
    };
    let fraction = move |read: fn(&Overview) -> Option<f64>| move || overview.with(read);
    let gauge = tile.gauge.map(|gauge| match gauge {
        Gauge::Health => bar(GAUGE, fraction(|o| o.margin_ratio), move || {
            health_color(overview.with(|o| o.margin_ratio))
        }),
        Gauge::Usage => bar(GAUGE, fraction(|o| o.margin_usage), || theme::ACCENT.get()),
        Gauge::Tier => bar(GAUGE, fraction(|o| o.tier_progress), || theme::ACCENT.get()),
        Gauge::Sides => {
            let side = move |pick: fn((f64, f64)) -> f64| {
                move || overview.with(|o| o.sides.map_or(0.0, pick)) as f32
            };
            track(GAUGE)
                .child(view().flex_grow(side(|(long, _)| long)).bg(theme::POSITIVE))
                .child(
                    view()
                        .flex_grow(side(|(_, short)| short))
                        .bg(theme::NEGATIVE),
                )
        }
    });
    let tile_view = view()
        .test_id(&format!(
            "{ID}-tile-{}",
            tile.title
                .to_lowercase()
                .replace(" & ", "-")
                .replace(' ', "-")
        ))
        .min_w(px(0.0))
        .overflow_hidden()
        .flex_col()
        .gap(px(4.0))
        .px(px(PADDING))
        .py(px(10.0))
        .border_r_1()
        .border_b_1()
        .border_color(theme::BORDER)
        .child(title);
    let tile_view = match gauge {
        Some(gauge) => tile_view.child(view().py(px(4.0)).child(gauge)),
        None => tile_view,
    };
    tile_view.children(
        tile.rows
            .iter()
            .map(|figure| line(overview, *figure))
            .collect::<Vec<_>>(),
    )
}

/// `figure`'s row in a tile: its name on the left, what it reads on the
/// right.
fn line(overview: Signal<Overview>, figure: Figure) -> View {
    let row = view();
    let row = match figure.hint() {
        Some(hint) => row.tooltip(ui::hint(move || hint.to_owned())),
        None => row,
    };
    row.test_id(&format!("{ID}-{}", figure.id()))
        .h(px(LINE_HEIGHT))
        .flex_none()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            text(figure.label())
                .select_text()
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT_MUTED),
        )
        .child(
            value(overview, figure)
                .test_id(&format!("{ID}-{}-value", figure.id()))
                .font_weight(FontWeight::MEDIUM),
        )
}

/// The balances: the titles (while there is a row), then a row per asset.
fn table(holdings: Signal<Shared<Vec<Holding>>>) -> View {
    let untitled = move || holdings.with(|holdings| holdings.0.is_empty());
    let titles = Column::ALL.map(|column| {
        cell(column)
            .h(px(TITLE_HEIGHT))
            .child(heading(column.title()))
            .style_when(untitled, |s| s.hidden())
    });
    let lines = each(
        move || holdings.with(|holdings| Vec::clone(&holdings.0)),
        |holding: &Holding| holding.key.clone(),
        move |holding, _| {
            Column::ALL
                .iter()
                .map(|column| {
                    let column = *column;
                    let figure = text(move || holding.with(|holding| column.figure(holding)))
                        .min_w(px(0.0))
                        .truncate()
                        .tabular_nums();
                    let figure = match column {
                        Column::Asset => figure.select_text().font_weight(FontWeight::SEMIBOLD),
                        Column::Pnl => figure
                            .select_text()
                            .text_color(move || Tone::color(holding.with(|holding| holding.tone))),
                        _ => figure.select_text(),
                    };
                    // A row keeps its account: it is in its key.
                    let owner = match column {
                        Column::Asset => holding.with_untracked(|holding| holding.owner.clone()),
                        _ => None,
                    };
                    cell(column)
                        .h(px(ROW_HEIGHT))
                        .gap(px(6.0))
                        .children(owner.as_ref().map(mark_view))
                        .child(figure)
                })
                .collect::<Vec<_>>()
        },
    );
    view()
        .test_id(&format!("{ID}-holdings"))
        .w_full()
        .grid()
        .grid_template_columns(COLUMNS)
        .children(titles)
        .child(lines)
}

/// What every cell of the balances is: its content in the middle of its
/// height, at its right edge unless it is the asset's, and a line under
/// it.
fn cell(column: Column) -> View {
    let cell = view()
        .min_w(px(0.0))
        .flex()
        .items_center()
        .px(px(PADDING))
        .border_b_1()
        .border_color(theme::BORDER);
    match column {
        Column::Asset => cell,
        _ => cell.justify_end(),
    }
}
