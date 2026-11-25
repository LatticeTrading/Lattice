//! The connection status: the bar's indicator and the panel it opens (the
//! old app's `ConnectionStatusIndicator` and `ConnectionStatusPopover`).
//!
//! - The indicator is drawn in the color of the worst source's health
//!   ([`StatusBoard::overall`]): quiet while nothing is asked for, green,
//!   amber, red. Resting the pointer on it opens the panel as a preview,
//!   which goes when the pointer leaves both; a press keeps it open until
//!   it is closed (its close button, a press on the indicator or outside,
//!   Escape).
//! - The panel lists every source (Ferris's socket and its requests today)
//!   under a heading that folds, and under each what it carries: every
//!   topic subscribed to and every request polled, with the widgets that
//!   want it, how it is doing and when its data last came. Its footer
//!   copies a report of what is wrong.
//!
//! Nothing is kept for the panel while it is closed. Open, it reads the
//! sources themselves once a second, and at once when a source's health
//! changes ([`DataServices::connections`]); what it says of them is worked
//! out in [`model`]. The rows are made again only when the feeds or who
//! wants them change; how each is doing is bound to the sample.

mod model;

use std::{collections::HashMap, rc::Rc, time::Duration};

use nsh_core::{
    LineHeight,
    element::ImageSrc,
    overlay::{Align, Placement},
    prelude::*,
    time::unix_ms,
};

use self::model::{PAGE_SIZE, Row, RowState, Severity, Snapshot, last_page};
use super::icon_segment;
use crate::{
    data::{
        DataServices, Shared,
        net::{Consumer, StatusBoard},
    },
    layout,
    shell::clock::{self, Clock},
    storage::StoredWorkspace,
    theme, ui,
    widgets::common::{LogoStyle, exchange_logo},
};

/// The panel's widest and tallest (the old popover's).
const PANEL_WIDTH: f32 = 480.0;
const PANEL_HEIGHT: f32 = 600.0;
/// The header's height, and a source heading's.
const HEAD_HEIGHT: f32 = 40.0;
/// A panel button's height (the footer's, the pages').
const BUTTON_HEIGHT: f32 = 36.0;
/// How long the pointer rests on the indicator before the preview opens,
/// and how long after it left before the preview goes.
const HOVER_OPEN: Duration = Duration::from_millis(150);
const HOVER_CLOSE: Duration = Duration::from_millis(200);
/// How a row draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT_MUTED,
    initials_size: 7.0,
    boxed: false,
};

const CHEVRON_RIGHT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12" fill="none"><path d="m4.5 3 3 3-3 3" stroke="currentColor" stroke-width="1.25"/></svg>"#;
const CHEVRON_DOWN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12" fill="none"><path d="m3 4.5 3 3 3-3" stroke="currentColor" stroke-width="1.25"/></svg>"#;
const CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="m3 3 8 8M11 3l-8 8" stroke="currentColor" stroke-width="1.4"/></svg>"#;

/// The indicator's state, which lasts as long as the bar.
#[derive(Clone, Copy)]
pub(super) struct Connections {
    open: Signal<bool>,
    /// A press opened it: it stays until it is closed. Otherwise it is a
    /// preview, which goes when the pointer leaves.
    pinned: Signal<bool>,
    over_indicator: Signal<bool>,
    over_panel: Signal<bool>,
    /// Counts the pointer's comings and goings and the presses: a hover
    /// timer acts only when none came since it was set.
    moves: Signal<u64>,
    /// The indicator, which the panel opens below.
    anchor: ElementRef,
}

impl Connections {
    pub(super) fn new() -> Self {
        Self {
            open: signal(false),
            pinned: signal(false),
            over_indicator: signal(false),
            over_panel: signal(false),
            moves: signal(0),
            anchor: element_ref(),
        }
    }

    /// Whether the panel shows (tracked).
    pub(super) fn is_open(self) -> bool {
        self.open.get()
    }

    fn close(self) {
        self.moved();
        batch(|| {
            self.open.set(false);
            self.pinned.set(false);
            // A panel that is gone hears no leave.
            self.over_panel.set(false);
        });
    }

    /// A press on the indicator: keeps the panel open, or closes the one a
    /// press kept open.
    fn press(self) {
        if self.pinned.get_untracked() {
            self.close();
        } else {
            self.moved();
            batch(|| {
                self.open.set(true);
                self.pinned.set(true);
            });
        }
    }

    /// Notes that the pointer or a press changed things, which makes the
    /// pending hover timer stale; the new count.
    fn moved(self) -> u64 {
        let next = self.moves.get_untracked().wrapping_add(1);
        self.moves.set(next);
        next
    }

    fn enter_indicator(self) {
        self.over_indicator.set(true);
        let at = self.moved();
        if self.open.get_untracked() {
            return;
        }
        set_timeout(HOVER_OPEN, move || {
            if self.moves.get_untracked() == at && !self.open.get_untracked() {
                self.open.set(true);
            }
        });
    }

    fn leave_indicator(self) {
        self.over_indicator.set(false);
        self.left();
    }

    fn enter_panel(self) {
        self.over_panel.set(true);
        self.moved();
    }

    fn leave_panel(self) {
        self.over_panel.set(false);
        self.left();
    }

    /// The pointer left the indicator or the panel: a preview goes unless
    /// it comes back to either in time.
    fn left(self) {
        let at = self.moved();
        if !self.open.get_untracked() || self.pinned.get_untracked() {
            return;
        }
        set_timeout(HOVER_CLOSE, move || {
            let stale = self.moves.get_untracked() != at
                || self.pinned.get_untracked()
                || self.over_indicator.get_untracked()
                || self.over_panel.get_untracked();
            if !stale {
                self.close();
            }
        });
    }

    /// The bar's indicator, `width` wide: the icon in the color of the
    /// sources' health. Without a `status` to read (no data services above
    /// the bar) it is the icon alone and opens nothing.
    pub(super) fn indicator(self, width: f32, status: Option<StatusBoard>) -> View {
        let severity =
            move || status.map_or(Severity::Neutral, |board| Severity::of(board.overall()));
        let open = self.open;
        let icon = view()
            .size(px(14.0))
            .pointer_events(PointerEvents::None)
            .child(
                svg(ImageSrc::svg_markup(include_str!(
                    "../../../assets/icons/connection-status.svg"
                )))
                .size_full()
                .text_color(move || severity().role().get()),
            );
        icon_segment(width)
            .element_ref(&self.anchor)
            .test_id("connection-status")
            .role(AccessibilityRole::Button)
            .label(move || format!("Connection status: {}", severity().word()))
            .accessibility_state(move || AccessibilityState {
                expanded: Some(open.get()),
                has_popup: true,
                ..Default::default()
            })
            .disabled(status.is_none())
            .text_color(move || severity().role().get())
            .hover(|style| style.bg(theme::PANEL))
            .style_when(move || open.get(), |style| style.bg(theme::PANEL))
            .on_mouse_enter(move |_: &MouseEvent| self.enter_indicator())
            .on_mouse_leave(move |_: &MouseEvent| self.leave_indicator())
            .on_press(move || self.press())
            .child(icon)
    }

    /// The panel, below the indicator and lined up with its right edge.
    /// `now` is the bar's ticking second, which paces the samples.
    pub(super) fn panel(
        self,
        services: DataServices,
        workspace: Signal<StoredWorkspace>,
        clock: Clock,
        now: Signal<i64>,
    ) -> Overlay {
        let board = services.status;
        let snapshot = computed(move || {
            // Each second, and at once when a source's health changes; the
            // widgets' names when the workspace does.
            now.get();
            board.overall();
            let names = workspace.with(names);
            let prefs = clock.get();
            untracked(|| {
                Shared::new(Snapshot::of(
                    services.connections(),
                    &names,
                    unix_ms(),
                    &|ms| clock::format_time(seconds(ms), prefs),
                ))
            })
        });
        // What is listed and under which source: the rows are made again
        // only when this changes.
        let rows = computed(move || snapshot.with(|snapshot| snapshot.0.rows.clone()));
        let sources = computed(move || {
            snapshot.with(|snapshot| {
                snapshot
                    .0
                    .sources
                    .iter()
                    .map(|source| (source.id, source.name, source.protocol))
                    .collect::<Vec<_>>()
            })
        });
        let panel = Panel {
            connections: self,
            snapshot,
            rows,
            clock,
            page: signal(0),
            expanded: signal(Vec::new()),
            copied: signal(false),
        };

        let sections = dynamic(move || {
            sources
                .get()
                .into_iter()
                .enumerate()
                .map(|(index, (id, name, protocol))| panel.section(index, id, name, protocol))
                .collect::<Vec<_>>()
        });
        let content = scroll()
            .flex_none()
            .max_h(move || px((panel_height() - HEAD_HEIGHT - BUTTON_HEIGHT - 4.0).max(0.0)))
            .child(
                view()
                    .w_full()
                    .flex_col()
                    .child(sections)
                    .child(panel.pages()),
            );

        let body = view()
            .test_id("connection-status-panel")
            .label("Connection status")
            .w(move || px((window().width.0 - 16.0).clamp(0.0, PANEL_WIDTH)))
            .max_h(move || px(panel_height()))
            .flex_col()
            .bg(theme::PANEL)
            .border_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT)
            .text_size(px(12.0))
            // The old popover's line height, which every text in it takes.
            .line_height(LineHeight::Multiple(1.5))
            .on_mouse_enter(move |_: &MouseEvent| self.enter_panel())
            .on_mouse_leave(move |_: &MouseEvent| self.leave_panel())
            .child(panel.header())
            .child(content)
            .child(panel.footer());

        // In an overlay, so it takes its presses wherever it hangs
        // (`ui::floating_panel` says why).
        overlay()
            .anchor(&self.anchor)
            .placement(Placement::Bottom)
            .align(Align::End)
            .offset(px(4.0))
            .margin(px(8.0))
            .on_press_outside(move |_: &Point<Points>| self.close())
            .on_cancel(move || self.close())
            .child(body)
    }
}

/// The panel's tallest in this window.
fn panel_height() -> f32 {
    (window().height.0 - 64.0).clamp(0.0, PANEL_HEIGHT)
}

/// A Unix time in milliseconds, in whole seconds.
fn seconds(ms: f64) -> i64 {
    (ms / 1_000.0).floor() as i64
}

/// What each widget in the workspace is called, by the consumer it is to
/// the data it asks for.
fn names(workspace: &StoredWorkspace) -> HashMap<Consumer, &'static str> {
    layout::widget_titles(workspace)
        .into_iter()
        .map(|(widget, title)| (Consumer::from(widget), title))
        .collect()
}

/// An open panel: the sample it shows and what the user did in it. Made
/// each time it opens, so it opens folded on its first page.
#[derive(Clone, Copy)]
struct Panel {
    connections: Connections,
    snapshot: Memo<Shared<Snapshot>>,
    /// Every feed's row, in list order.
    rows: Memo<Rc<Vec<Row>>>,
    clock: Clock,
    /// The page of rows shown, from 0 (kept within the pages there are).
    page: Signal<usize>,
    /// The sources unfolded, by id.
    expanded: Signal<Vec<&'static str>>,
    /// Whether a report was copied since it opened.
    copied: Signal<bool>,
}

impl Panel {
    /// A time of day on the bar's clock.
    fn time(self, ms: f64) -> String {
        clock::format_time(seconds(ms), self.clock.get())
    }

    /// The page shown: the one chosen, or the last when there are fewer.
    fn current_page(self) -> usize {
        self.page
            .get()
            .min(self.rows.with(|rows| last_page(rows.len())))
    }

    fn is_expanded(self, source: &'static str) -> bool {
        self.expanded.with(|expanded| expanded.contains(&source))
    }

    fn toggle(self, source: &'static str) {
        self.expanded.update(
            |expanded| match expanded.iter().position(|id| *id == source) {
                Some(at) => {
                    expanded.remove(at);
                }
                None => expanded.push(source),
            },
        );
    }

    /// Shows `page`, unfolding the sources with rows on it.
    fn show_page(self, page: usize) {
        let (rows, snapshot) = (self.rows.get_untracked(), self.snapshot.get_untracked());
        batch(|| {
            self.page.set(page);
            self.expanded.update(|expanded| {
                for row in rows.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
                    if let Some(source) = snapshot.0.sources.get(row.source)
                        && !expanded.contains(&source.id)
                    {
                        expanded.push(source.id);
                    }
                }
            });
        });
    }

    /// The header: how everything is doing, how many sources are asked for
    /// something, how many feeds are doing well, and the close button.
    fn header(self) -> View {
        let snapshot = self.snapshot;
        let connections = self.connections;
        let cell = || {
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .justify_center()
                .p(px(8.0))
        };
        let overall = status(
            move || snapshot.with(|snapshot| snapshot.0.severity),
            move || snapshot.with(|snapshot| snapshot.0.severity.title().to_owned()),
            // The header's own weight, as its other cells.
            FontWeight::MEDIUM,
        );
        let sources = text(move || {
            let active = snapshot.with(|snapshot| snapshot.0.active_sources());
            format!(
                "{active} {}",
                if active == 1 { "source" } else { "sources" }
            )
        })
        .number_of_lines(1)
        .tabular_nums();
        let feeds = text(move || {
            snapshot
                .with(|snapshot| format!("{}/{} feeds", snapshot.0.online(), snapshot.0.rows.len()))
        })
        .number_of_lines(1)
        .tabular_nums();
        let close = view()
            .w(px(HEAD_HEIGHT))
            .flex_none()
            .flex_row()
            .items_center()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT_MUTED)
            .role(AccessibilityRole::Button)
            .label("Close connection status")
            .hover(|style| {
                style
                    .bg(theme::ACCENT.opacity(0.06))
                    .text_color(theme::TEXT)
            })
            .active(|style| style.bg(theme::BORDER))
            .on_press(move || connections.close())
            .child(
                view()
                    .size(px(14.0))
                    .pointer_events(PointerEvents::None)
                    .child(svg(ImageSrc::svg_markup(CLOSE)).size_full()),
            );
        view()
            .min_h(px(HEAD_HEIGHT))
            .flex_none()
            .flex_row()
            .items_stretch()
            .bg(theme::SURFACE)
            .border_b_1()
            .border_color(theme::BORDER)
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .child(
                cell()
                    .flex_1()
                    .min_w(px(0.0))
                    .justify_start()
                    .child(overall),
            )
            .child(
                cell()
                    .border_l_1()
                    .border_color(theme::BORDER)
                    .child(sources),
            )
            .child(cell().border_l_1().border_color(theme::BORDER).child(feeds))
            .child(close)
    }

    /// One source: its heading, which folds it, and under it what is wrong
    /// with it and its feeds on the page shown.
    fn section(
        self,
        index: usize,
        id: &'static str,
        name: &'static str,
        protocol: &'static str,
    ) -> View {
        let snapshot = self.snapshot;
        let expanded = move || self.is_expanded(id);
        let latency = computed(move || {
            snapshot.with(|snapshot| {
                snapshot
                    .0
                    .sources
                    .get(index)
                    .and_then(|source| source.latency_ms)
            })
        });
        let meta = |content: Text| {
            content
                .flex_none()
                .number_of_lines(1)
                .text_size(px(10.0))
                .font_weight(FontWeight::NORMAL)
                .text_color(theme::TEXT_FAINT)
        };
        let identity = view()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(
                svg(move || {
                    ImageSrc::svg_markup(if expanded() {
                        CHEVRON_DOWN
                    } else {
                        CHEVRON_RIGHT
                    })
                })
                .size(px(12.0))
                .flex_none()
                .text_color(theme::TEXT_FAINT),
            )
            .child(text(name).min_w(px(0.0)).truncate())
            .child(meta(text(protocol)))
            .child(
                meta(text(move || {
                    latency
                        .get()
                        .map_or_else(String::new, |latency| format!("{latency} ms"))
                }))
                .tabular_nums()
                .style_when(move || latency.get().is_none(), |style| style.hidden()),
            );
        let state = status(
            move || {
                snapshot.with(|snapshot| {
                    snapshot
                        .0
                        .sources
                        .get(index)
                        .map_or(Severity::Neutral, |source| source.severity)
                })
            },
            move || {
                snapshot.with(|snapshot| {
                    snapshot
                        .0
                        .sources
                        .get(index)
                        .map_or_else(String::new, |source| source.status.to_owned())
                })
            },
            FontWeight::NORMAL,
        );
        let heading = view()
            .test_id(&format!("connection-source-{id}"))
            .w_full()
            .min_h(px(HEAD_HEIGHT))
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme::SURFACE)
            .text_color(theme::TEXT)
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .role(AccessibilityRole::Button)
            .label(format!("{name} {protocol}"))
            .accessibility_state(move || AccessibilityState {
                expanded: Some(expanded()),
                ..Default::default()
            })
            .hover(|style| style.bg(theme::ACCENT.opacity(0.06)))
            .active(|style| style.bg(theme::BORDER))
            .on_press(move || self.toggle(id))
            .child(identity)
            .child(state.flex_none());

        let body = dynamic(move || {
            expanded().then(|| {
                view()
                    .w_full()
                    .flex_col()
                    .child(self.notices(index))
                    .child(self.feeds(index))
                    .child(self.notes(index))
            })
        });
        view()
            .w_full()
            .flex_col()
            .border_b_1()
            .border_color(theme::BORDER)
            .child(heading)
            .child(body)
    }

    /// What is wrong with source `index`, in the warning color; nothing
    /// while it is well.
    fn notices(self, index: usize) -> impl IntoElement {
        let snapshot = self.snapshot;
        let lines = computed(move || {
            snapshot.with(|snapshot| {
                snapshot
                    .0
                    .sources
                    .get(index)
                    .map(|source| source.notices.clone())
                    .unwrap_or_default()
            })
        });
        dynamic(move || {
            let lines = lines.get();
            (!lines.is_empty()).then(|| {
                view()
                    .w_full()
                    .flex_col()
                    .gap(px(2.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(theme::BORDER)
                    .text_size(px(11.0))
                    .text_color(theme::WARNING)
                    .role(AccessibilityRole::Alert)
                    .children(lines.into_iter().map(text))
            })
        })
    }

    /// Source `index`'s quiet lines, after its feeds.
    fn notes(self, index: usize) -> impl IntoElement {
        let snapshot = self.snapshot;
        let lines = computed(move || {
            snapshot.with(|snapshot| {
                snapshot
                    .0
                    .sources
                    .get(index)
                    .map(|source| source.notes.clone())
                    .unwrap_or_default()
            })
        });
        dynamic(move || lines.get().into_iter().map(note).collect::<Vec<_>>())
    }

    /// Source `index`'s feeds on the page shown, or a line saying it has
    /// none.
    fn feeds(self, index: usize) -> impl IntoElement {
        dynamic(move || {
            let rows = self.rows.get();
            let start = self.current_page() * PAGE_SIZE;
            let shown: Vec<View> = rows
                .iter()
                .skip(start)
                .take(PAGE_SIZE)
                .filter(|row| row.source == index)
                .map(|row| self.feed(row))
                .collect();
            if !shown.is_empty() {
                return shown;
            }
            let elsewhere = rows.iter().any(|row| row.source == index);
            vec![note(
                if elsewhere {
                    "No feeds on this page."
                } else {
                    "No active feeds."
                }
                .to_owned(),
            )]
        })
    }

    /// One feed: what it is and how it is doing, what is asked of it, then
    /// who wants it and when its data last came.
    fn feed(self, row: &Row) -> View {
        let snapshot = self.snapshot;
        let id = row.id.clone();
        let state = computed(move || {
            snapshot.with(|snapshot| snapshot.0.states.get(&id).cloned().unwrap_or_default())
        });
        let mut pair = view()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(6.0));
        if !row.exchange.is_empty() {
            pair = pair.child(exchange_logo(&row.exchange, LOGO));
        }
        let pair = pair.child(
            text(row.symbol.clone())
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM),
        );
        let doing = status(
            move || state.with(|state| state.severity),
            move || state.with(|state| state.label.to_owned()),
            FontWeight::NORMAL,
        );
        let last = text(move || match state.with(|state| state.activity_ms) {
            Some(at) => format!("Last {}", self.time(at)),
            None => "—".to_owned(),
        })
        .flex_none()
        .number_of_lines(1)
        .tabular_nums()
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT);
        let line = || view().w_full().flex_row().items_center().gap(px(12.0));
        let exchange = match row.exchange.as_str() {
            "" => String::new(),
            id => format!("{} ", crate::exchanges::display_name(id)),
        };
        view()
            .w_full()
            .flex_col()
            .gap(px(2.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(theme::BORDER)
            .label(format!("{exchange}{}, {}", row.symbol, row.description))
            .tooltip(ui::hint(move || state.with(RowState::hint)).style_when(
                move || state.with(|state| state.hint().is_empty()),
                |style| style.hidden(),
            ))
            .child(line().child(pair).child(doing.flex_none()))
            .child(
                text(row.description.clone())
                    .truncate()
                    .text_size(px(10.0))
                    .text_color(theme::TEXT_FAINT),
            )
            .child(
                line()
                    .child(
                        text(row.consumers.clone())
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(theme::TEXT_MUTED),
                    )
                    .child(last),
            )
    }

    /// Previous, which rows show, and Next: only while there is more than
    /// one page.
    fn pages(self) -> impl IntoElement {
        dynamic(move || {
            let count = self.rows.with(|rows| rows.len());
            let last = last_page(count);
            if last == 0 {
                return None;
            }
            let page = self.current_page();
            let start = page * PAGE_SIZE;
            let range = format!(
                "{}–{} of {count}",
                start + 1,
                (start + PAGE_SIZE).min(count)
            );
            Some(
                view()
                    .w_full()
                    .flex_none()
                    .flex_row()
                    .items_stretch()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .role(AccessibilityRole::Navigation)
                    .label("Feed pages")
                    .child(
                        action("Previous")
                            .border_r_1()
                            .border_color(theme::BORDER)
                            .disabled(page == 0)
                            .on_press(move || self.show_page(page.saturating_sub(1))),
                    )
                    .child(
                        text(range)
                            .self_center()
                            .px(px(8.0))
                            .number_of_lines(1)
                            .tabular_nums()
                            .text_size(px(11.0))
                            .text_color(theme::TEXT_FAINT),
                    )
                    .child(
                        action("Next")
                            .border_l_1()
                            .border_color(theme::BORDER)
                            .disabled(page == last)
                            .on_press(move || self.show_page((page + 1).min(last))),
                    ),
            )
        })
    }

    /// When the sample was taken, and the button that copies the report.
    fn footer(self) -> View {
        let (snapshot, copied) = (self.snapshot, self.copied);
        let as_of = view()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(12.0))
            .py(px(6.0))
            .text_size(px(10.0))
            .text_color(theme::TEXT_FAINT)
            .child(
                text(move || {
                    let at = snapshot.with(|snapshot| snapshot.0.captured_ms);
                    format!("As of {}", self.time(at))
                })
                .number_of_lines(1)
                .tabular_nums(),
            )
            .child(
                text("· Report copied")
                    .number_of_lines(1)
                    .role(AccessibilityRole::Alert)
                    .style_when(move || !copied.get(), |style| style.hidden()),
            );
        let copy = action("Copy report")
            .border_l_1()
            .border_color(theme::BORDER)
            .text_size(px(11.0))
            .tooltip(ui::hint(|| {
                "Copy current connection problems, affected pairs and components, and key timings"
                    .to_owned()
            }))
            .on_press(move || {
                let written =
                    set_clipboard(snapshot.with_untracked(|snapshot| snapshot.0.report()));
                // Said only once the clipboard took it (NSH 0.7 answers).
                spawn(async move {
                    copied.set(written.await != ClipboardWrite::Failed);
                });
            });
        view()
            .w_full()
            .flex_none()
            .flex_row()
            .items_stretch()
            .border_t_1()
            .border_color(theme::BORDER)
            .child(as_of)
            .child(copy)
    }
}

/// A dot and a word in a severity's color: how something is doing.
fn status(
    severity: impl Fn() -> Severity + Clone + 'static,
    label: impl Fn() -> String + 'static,
    weight: FontWeight,
) -> View {
    let dot = severity.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            view()
                .size(px(6.0))
                .flex_none()
                .rounded_full()
                .bg(move || dot().role().get()),
        )
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(11.0))
                .font_weight(weight)
                .text_color(move || severity().role().get()),
        )
}

/// A quiet line under a source's feeds.
fn note(line: String) -> View {
    view()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
        .child(text(line))
}

/// One of the panel's flat text buttons (the footer's, the pages').
fn action(label: &str) -> View {
    view()
        .min_h(px(BUTTON_HEIGHT))
        .flex_none()
        .flex_row()
        .items_center()
        .justify_center()
        .px(px(12.0))
        .text_size(px(12.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .hover(|style| {
            style
                .bg(theme::ACCENT.opacity(0.06))
                .text_color(theme::TEXT)
        })
        .active(|style| style.bg(theme::BORDER))
        .disabled_style(|style| style.opacity(0.55).text_color(theme::TEXT_FAINT))
        .child(text(label.to_owned()))
}
