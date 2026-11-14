//! The Everything Bar's symbol search and its link color (the old app's
//! `EverythingBar` search, `searchMarketUniverse` and color link selector).
//!
//! - The link button, left of the field, shows the bar's link color
//!   ([`ActiveLink`]) and opens a dropdown of the five colors, each with
//!   how many widgets are linked to it. (The old bar showed a pill per
//!   color in use and could have none selected, which sent picks to a
//!   global symbol; nothing in Lattice follows one.)
//! - The field opens its dropdown when it takes the focus or is typed in.
//!   With nothing typed the dropdown lists the most traded markets of every
//!   exchange, each once its volumes are known; typing searches every
//!   market at once ([`Directory::search`]).
//! - A pick (a press, or Enter on the keyboard's row, which Up, Down and
//!   the pointer move) shows the market on every widget linked to the
//!   bar's color ([`layout::link_market`]), clears the field and closes
//!   the dropdown. Escape does the last two; a press outside closes it and
//!   keeps what is typed. While no widget is linked to the bar's color, the
//!   dropdown says so.
//!
//! Data: the app's market directory ([`crate::data::Markets`]). The bar
//! holds a lease on it for as long as it lives, so the catalogs are there
//! before the field is first focused (Hyperliquid's alone takes 10-18s) and
//! every market picker opens on them. The volumes the results are ranked by
//! are polled only while the dropdown is open.
//!
//! The dropdown is an overlay below the field (`ui::floating_panel` says
//! why it must be one). Its rows are the old search result's blocks: 34pt,
//! the exchange's logo in a cell of its own, the pair, then its detail and
//! its type in cells at the far edge. They are a `virtual_list` on
//! [`RowScroll`], so none shows blank.

use std::{cell::RefCell, rc::Rc};

use nsh_core::{
    element::ImageSrc,
    keymap::When,
    overlay::{Align, Placement},
    prelude::*,
};

use crate::{
    data::{DataServices, Markets, Shared, ferris::CatalogMarket, markets::Directory},
    exchanges, layout,
    link::ActiveLink,
    storage::StoredWorkspace,
    theme,
    ui::{self, RowScroll, Side},
    widgets::common::{LogoStyle, bound_exchange_logo, tag_text},
};

/// A result's height (the old `.search-result-item`).
const ROW_HEIGHT: f32 = 34.0;
/// The results' tallest (the old dropdown's `max-height`).
const LIST_HEIGHT: f32 = 400.0;
/// The room a message takes while there are no results.
const MESSAGE_HEIGHT: f32 = 50.0;
/// The dropdown's narrowest, so a narrow field's results are still whole.
const MIN_WIDTH: f32 = 380.0;
/// The search icon's cell, left of the field.
const ICON_CELL: f32 = 32.0;
/// A result's exchange cell, and the type cell at its other end.
const LOGO_CELL: f32 = 40.0;
const TYPE_CELL: f32 = 52.0;
/// The keyboard row's mark at its left edge.
const ROW_EDGE: f32 = 2.0;
/// How a result draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 16.0,
    color: theme::TEXT_MUTED,
    initials_size: 8.0,
    boxed: false,
};

/// What Enter in the field does, which the dropdown says while it shows.
type Enter = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// The search's state, which lasts as long as the bar.
#[derive(Clone, Copy)]
struct Search {
    workspace: Signal<StoredWorkspace>,
    /// The bar's link color: where a pick goes.
    link: ActiveLink,
    /// What is typed.
    query: Signal<String>,
    /// Whether the dropdown shows.
    open: Signal<bool>,
    /// The field, to take the focus off it.
    field: ElementRef,
    /// What the dropdown opens below: the icon and the field.
    anchor: ElementRef,
    /// The anchor's width, as laid out.
    width: Signal<f32>,
    /// How many widgets each link color has, in the colors' order.
    linked: Memo<[usize; 5]>,
}

impl Search {
    /// Closes the dropdown and clears the field, as Escape and a pick do.
    fn dismiss(self) {
        batch(|| {
            self.open.set(false);
            self.query.set(String::new());
        });
        self.field.blur();
    }
}

/// The bar's link button and its search field, side by side, filling the
/// room they are given.
pub(super) fn search(workspace: Signal<StoredWorkspace>) -> View {
    let link = ActiveLink::restore();
    let linked = computed(move || workspace.with(layout::link_counts));
    let search = Search {
        workspace,
        link,
        query: signal(String::new()),
        open: signal(false),
        field: element_ref(),
        anchor: element_ref(),
        width: signal(0.0_f32),
        linked,
    };
    // Held for as long as the bar lives: the markets are there before
    // anyone searches, and for every market picker.
    let markets = DataServices::try_get().map(|services| {
        let markets = services.markets;
        let directory = markets.lease().release_on_cleanup();
        (markets, directory)
    });

    let link_button = ui::link_button(
        "Search link color",
        move || Some(link.get()),
        false,
        move |choice| match choice.map(|color| linked.get()[color.index()]) {
            None | Some(0) => String::new(),
            Some(1) => "1 widget".to_owned(),
            Some(count) => format!("{count} widgets"),
        },
        move |choice| {
            if let Some(color) = choice {
                link.select(color);
            }
        },
        Side::Left,
        10.0,
    )
    .h_full()
    .w(px(40.0))
    .border_l_1()
    .border_color(theme::BORDER)
    .test_id("bar-link");

    let focused = signal(false);
    let enter = Enter::default();
    let field = {
        let enter = enter.clone();
        ui::text_field(search.query)
            .element_ref(&search.field)
            .test_id("symbol-search")
            .flex_1()
            .h_full()
            .pl(px(0.0))
            .pr(px(12.0))
            .text_size(px(12.0))
            .bg(theme::SURFACE)
            // The old field: lighter while it has the focus.
            .focus(|s| s.bg(theme::PANEL))
            .placeholder("Search symbols...")
            .label("Search symbols")
            // Enter picks; the pick takes the focus off, nothing else does.
            .submit_behavior(SubmitBehavior::Submit)
            .on_focus(move || {
                batch(|| {
                    focused.set(true);
                    search.open.set(true);
                });
            })
            .on_blur(move || focused.set(false))
            .on_change(move |_: &str| search.open.set(true))
            .on_submit(move |_: &str| {
                let pick = enter.borrow().clone();
                if let Some(pick) = pick {
                    pick();
                }
            })
    };
    // In a cell of its own rather than over the field: the field is the
    // platform's text box, which draws over what is laid on it.
    let icon = view()
        .h_full()
        .w(px(ICON_CELL))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(move || {
            if focused.get() {
                theme::PANEL.get()
            } else {
                theme::SURFACE.get()
            }
        })
        .text_color(theme::TEXT_FAINT)
        .on_press(move || search.field.focus())
        .child(
            svg(ImageSrc::svg_markup(include_str!(
                "../../../assets/icons/search.svg"
            )))
            .size(px(14.0)),
        );
    let field = view()
        .element_ref(&search.anchor)
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .border_l_1()
        .border_color(theme::BORDER)
        .on_layout(move |bounds: &Bounds<Points>| search.width.set(bounds.size.width.0))
        .child(icon)
        .child(field)
        // Its own `dynamic`: the field outlives every opening.
        .child(dynamic(move || {
            search
                .open
                .get()
                .then(|| dropdown(search, markets.clone(), enter.clone()))
        }));

    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .child(link_button)
        .child(field)
}

/// The dropdown, below the field and at least as wide: what it has to say,
/// then the results.
fn dropdown(
    search: Search,
    markets: Option<(Markets, Memo<Shared<Directory>>)>,
    enter: Enter,
) -> Overlay {
    let body = match markets {
        Some((markets, directory)) => results(search, markets, directory, enter),
        None => message(|| "Market data is unavailable.".to_owned()),
    };
    overlay()
        .anchor(&search.anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        // Under the bar's own border.
        .offset(px(1.0))
        // The focus goes with it, so the field opens it again when it is
        // next pressed, as it does when it takes the focus.
        .on_press_outside(move |_: &Point<Points>| {
            search.open.set(false);
            search.field.blur();
        })
        .on_cancel(move || search.dismiss())
        .child(
            view()
                .test_id("symbol-results")
                .w(move || px(search.width.get().max(MIN_WIDTH)))
                .flex_col()
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_color(theme::TEXT)
                .child(unlinked(search))
                .child(body),
        )
}

/// Says that no widget is linked to the bar's color, while none is: a pick
/// would change nothing.
fn unlinked(search: Search) -> View {
    let (link, linked) = (search.link, search.linked);
    view()
        .flex_none()
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Alert)
        .style_when(
            move || linked.get()[link.get().index()] > 0,
            |s| s.hidden(),
        )
        .child(text(move || {
            format!(
                "No widget is linked to {}. Link one from its header's link button to send it markets.",
                link.get().label()
            )
        }))
}

/// A line in place of results.
fn message(content: impl Fn() -> String + 'static) -> View {
    view()
        .px(px(12.0))
        .py(px(16.0))
        .text_center()
        .text_size(px(12.0))
        .text_color(theme::TEXT_FAINT)
        .role(AccessibilityRole::Text)
        .child(text(content))
}

/// What could not load, with Retry; hidden while everything did.
fn failures(directory: Memo<Shared<Directory>>, markets: Markets) -> View {
    let failed = computed(move || {
        directory.with(|directory| {
            let directory = &directory.0;
            if let Some(error) = &directory.error {
                return Some(format!(
                    "Couldn't load the exchange list: {}",
                    error.message
                ));
            }
            let first = directory
                .exchanges
                .iter()
                .find_map(|listed| listed.error.as_ref())?;
            let names: Vec<&str> = directory
                .exchanges
                .iter()
                .filter(|listed| listed.error.is_some())
                .map(|listed| exchanges::display_name(&listed.exchange))
                .collect();
            Some(format!(
                "Couldn't load {}: {}",
                names.join(", "),
                first.message
            ))
        })
    });
    view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .role(AccessibilityRole::Alert)
        .style_when(move || failed.with(Option::is_none), |s| s.hidden())
        .child(
            text(move || failed.get().unwrap_or_default())
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(11.0))
                .text_color(theme::DANGER),
        )
        .child(
            view()
                .min_h(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .px(px(12.0))
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT_MUTED)
                .role(AccessibilityRole::Button)
                .label("Retry loading markets")
                .hover(|s| s.border_color(theme::ACCENT).text_color(theme::TEXT))
                .on_press(move || markets.retry())
                .child(text("Retry")),
        )
}

/// The results for what is typed, and what a pick does. Made each time the
/// dropdown opens: the volumes are polled, and Up, Down and Enter are the
/// list's, only while it shows.
fn results(
    search: Search,
    markets: Markets,
    directory: Memo<Shared<Directory>>,
    enter: Enter,
) -> View {
    let query = search.query;
    let volumes = markets.lease().rank().release_on_cleanup();
    // What the list is ranked by: the volumes as they were when the search
    // last changed, or since completed. A mere refresh of them never moves
    // the rows under the pointer.
    let ranking = signal(volumes.get_untracked());
    effect(move || {
        query.with(|_| ());
        ranking.set(volumes.get_untracked());
    });
    effect(move || {
        let latest = volumes.get();
        if ranking.with_untracked(|held| held.0.pending() || held.0.is_empty()) {
            ranking.set(latest);
        }
    });
    let shown = computed(move || {
        let typed = query.get();
        directory.with(|directory| {
            ranking.with(|volumes| Shared::new(directory.0.search(&volumes.0, &typed)))
        })
    });
    let count = computed(move || shown.with(|shown| shown.0.len()));
    let empty = move || {
        let typed = query.with(|typed| typed.trim().to_owned());
        let waiting = directory.with(|directory| directory.0.loading())
            || (typed.is_empty() && ranking.with(|volumes| volumes.0.pending()));
        if waiting {
            "Loading symbols...".to_owned()
        } else if typed.is_empty() {
            "No symbols available".to_owned()
        } else {
            format!("No symbols found for \"{typed}\"")
        }
    };

    let list = element_ref();
    let scroll = RowScroll::new(list, ROW_HEIGHT, move || count.get());
    // The keyboard's row: the first after every new search or result count.
    let highlight = signal(0_usize);
    let reset = computed(move || (query.get(), count.get()));
    effect(move || {
        reset.with(|_| ());
        highlight.set(0);
        scroll.to(0.0);
    });

    let pick: Rc<dyn Fn(usize)> = Rc::new(move |index| {
        let Some(market) = shown.with_untracked(|shown| shown.0.get(index).cloned()) else {
            return;
        };
        let mut state = search.workspace.get_untracked();
        let color = search.link.get_untracked();
        if layout::link_market(&mut state, color, &market.selection) > 0 {
            layout::persist(&state);
            search.workspace.set(state);
        }
        search.dismiss();
    });
    let step = move |by: isize| {
        let count = count.get_untracked();
        if count == 0 {
            return;
        }
        let next = highlight
            .get_untracked()
            .saturating_add_signed(by)
            .min(count - 1);
        highlight.set(next);
        scroll.reveal(next);
    };
    let keys = KeyOptions {
        when: When::Always,
        repeat: true,
        ..KeyOptions::default()
    };
    on_key_with("down", keys, move |_| step(1));
    on_key_with("up", keys, move |_| step(-1));
    {
        let pick = pick.clone();
        *enter.borrow_mut() = Some(Rc::new(move || pick(highlight.get_untracked())));
        cleanup(move || {
            enter.borrow_mut().take();
        });
    }

    let rows = scroll
        .drive(virtual_list(
            move || count.get(),
            move |at: Position| row(at, shown, highlight, pick.clone()),
        ))
        .estimated_item_size(px(ROW_HEIGHT))
        .element_ref(&list)
        .flex_1()
        .min_h(px(0.0))
        .role(AccessibilityRole::Listbox)
        .label("Markets")
        .empty(message(empty));
    let height = move || match count.get() {
        0 => MESSAGE_HEIGHT,
        count => (count as f32 * ROW_HEIGHT).min(LIST_HEIGHT),
    };
    view().flex_col().child(failures(directory, markets)).child(
        view()
            .h(move || px(height()))
            .flex_none()
            .relative()
            .flex_col()
            .child(rows)
            .child(
                scroll
                    .scrollbar("Scroll markets")
                    .absolute()
                    .top(px(0.0))
                    .bottom(px(0.0))
                    .right(px(0.0)),
            ),
    )
}

/// One result, in the old search result's blocks: its exchange's logo in a
/// cell, its pair, then its detail (`USDT-M`, an expiry) and its type
/// (`PERP`) in cells at the far edge. Shaded with a bar at its left edge
/// while it is the keyboard's row, which the pointer moves too. The last
/// row has no divider under it.
fn row(
    at: Position,
    shown: Memo<Shared<Vec<Rc<CatalogMarket>>>>,
    highlight: Signal<usize>,
    pick: Rc<dyn Fn(usize)>,
) -> View {
    let market = computed(move || shown.with(|shown| shown.0.get(at.index()).cloned().map(Shared)));
    let field = move |read: fn(&CatalogMarket) -> String| {
        move || market.with(|m| m.as_ref().map(|m| read(&m.0)).unwrap_or_default())
    };
    let exchange = move || market.with(|m| m.as_ref().map(|m| m.0.selection.exchange.clone()));
    let detail = move || market.with(|m| m.as_ref().and_then(|m| m.0.detail.clone()));
    let highlighted = move || highlight.get() == at.index();
    let last = move || at.index() + 1 == shown.with(|shown| shown.0.len());
    let (logo, initials) = bound_exchange_logo(exchange, LOGO);
    let cell = || {
        view()
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .border_color(theme::BORDER)
    };
    view()
        .relative()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .border_b_1()
        .border_color(move || {
            if last() {
                theme::BORDER.opacity(0.0).get()
            } else {
                theme::BORDER.get()
            }
        })
        .bg(move || {
            if highlighted() {
                theme::SURFACE.get()
            } else {
                theme::PANEL.get()
            }
        })
        .role(AccessibilityRole::Option)
        .label(field(|m| {
            format!(
                "{} {} {}",
                exchanges::display_name(&m.selection.exchange),
                m.selection.symbol,
                m.tags
            )
        }))
        .accessibility_state(move || AccessibilityState {
            selected: highlighted(),
            ..Default::default()
        })
        .on_mouse_enter(move |_: &MouseEvent| highlight.set(at.index()))
        .on_press(move || pick(at.index()))
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(ROW_EDGE))
                .bg(theme::ACCENT)
                .style_when(move || !highlighted(), |s| s.hidden()),
        )
        .child(
            cell()
                .w(px(LOGO_CELL))
                .border_r_1()
                .child(logo)
                .child(initials),
        )
        .child(
            text(field(|m| m.selection.symbol.clone()))
                .flex_1()
                .min_w(px(0.0))
                .px(px(12.0))
                .truncate()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::TEXT),
        )
        .child(
            cell()
                .px(px(8.0))
                .border_l_1()
                .style_when(move || detail().is_none(), |s| s.hidden())
                .child(
                    tag_text(text(move || detail().unwrap_or_default())).text_color(theme::WARNING),
                ),
        )
        .child(
            cell().w(px(TYPE_CELL)).border_l_1().child(
                tag_text(text(field(|m| m.selection.kind.label().to_owned())))
                    .text_color(theme::TEXT_FAINT),
            ),
        )
}
