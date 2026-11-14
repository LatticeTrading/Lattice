//! The results list and what could not load: a virtual list of the shown
//! markets, scrolled by the app, and the failures with Retry.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{ROW_HEIGHT, listing::Listing, sources::Sources};
use crate::{
    data::ferris::{CatalogMarket, MarketSelection},
    exchanges, theme,
    ui::RowScroll,
    widgets::common::{Shared, market_tags},
};

/// The old modal's result list height.
const LIST_HEIGHT: f32 = 400.0;
/// The current selection's mark at a row's left edge.
const CURRENT_EDGE: f32 = 3.0;

/// What could not load, with Retry; hidden while everything did.
pub(super) fn failures(listing: Memo<Shared<Listing>>, sources: Sources) -> View {
    let message = computed(move || {
        listing.with(|listing| {
            let failed = &listing.0.failed;
            let (_, first) = failed.first()?;
            let names: Vec<&str> = failed.iter().map(|(name, _)| name.as_str()).collect();
            Some(format!(
                "Couldn't load {}: {}",
                names.join(", "),
                first.message
            ))
        })
    });
    // The old error message: centered red text, here with Retry under it.
    view()
        .flex_none()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .px(px(20.0))
        .py(px(40.0))
        .role(AccessibilityRole::Alert)
        .style_when(move || message.with(Option::is_none), |s| s.hidden())
        .child(
            text(move || message.get().unwrap_or_default())
                .text_center()
                .text_size(px(12.0))
                .text_color(theme::DANGER),
        )
        .child(
            view()
                .min_h(px(24.0))
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
                .on_press(move || sources.retry())
                .child(text("Retry")),
        )
}

/// The results' view state, shared with the picker: the list and its
/// scrolling, what it shows, the keyboard's row, whether rows name their
/// exchange, the current selection, and what a pick does.
pub(super) struct Results {
    pub list: ElementRef,
    pub scroll: RowScroll,
    pub shown: Memo<Shared<Vec<Rc<CatalogMarket>>>>,
    pub listing: Memo<Shared<Listing>>,
    /// The keyboard's row.
    pub highlight: Signal<usize>,
    /// Rows name their exchange (under "All" of a picker of every exchange).
    pub names: Memo<bool>,
    pub current: Option<Rc<MarketSelection>>,
    pub pick: Rc<dyn Fn(usize)>,
}

impl Results {
    /// The results, [`LIST_HEIGHT`] tall, scrolled by the app, with their
    /// scrollbar; a message instead while there are none.
    pub fn view(self) -> View {
        let (list, scroll, shown, listing) = (self.list, self.scroll, self.shown, self.listing);
        let state = Rc::new(self);
        let empty = move || {
            listing.with(|listing| {
                if listing.0.loading && listing.0.all.is_empty() {
                    "Loading symbols..."
                } else {
                    "No symbols found"
                }
            })
        };
        let rows = virtual_list(
            move || shown.with(|s| s.0.len()),
            move |at: Position| state.row(at),
        )
        .estimated_item_size(px(ROW_HEIGHT))
        .element_ref(&list)
        .scrollable(false)
        .scrollbar(false)
        .flex_1()
        .min_h(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| scroll.measure(bounds.size.height.0))
        .on_wheel(move |event: &WheelEvent| scroll.wheel(event))
        .role(AccessibilityRole::Listbox)
        .label("Markets")
        // The old message: centered, well clear of the edges.
        .empty(
            view()
                .px(px(20.0))
                .py(px(40.0))
                .text_center()
                .text_size(px(12.0))
                .text_color(theme::TEXT_MUTED)
                .role(AccessibilityRole::Text)
                .child(text(move || empty().to_owned())),
        );
        view()
            .h(px(LIST_HEIGHT))
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
            )
    }

    /// One result, as the old `.symbol-search-result`: its pair, its
    /// exchange's name under "All", its tags in boxes; shaded while it is the
    /// keyboard's row (the pointer moves that row too), and shaded with a bar
    /// at its left edge while it is the current selection. The last row has no
    /// divider under it.
    fn row(&self, at: Position) -> View {
        let (shown, highlight, names) = (self.shown, self.highlight, self.names);
        let (current, pick) = (self.current.clone(), self.pick.clone());
        let market = computed(move || shown.with(|s| s.0.get(at.index()).cloned().map(Shared)));
        let field = move |read: fn(&CatalogMarket) -> String| {
            move || market.with(|m| m.as_ref().map(|m| read(&m.0)).unwrap_or_default())
        };
        let is_current = computed(move || {
            market.with(|m| {
                m.as_ref()
                    .zip(current.as_ref())
                    .is_some_and(|(m, current)| current.same_market(&m.0.selection))
            })
        });
        let exchange = computed(move || {
            market.with(|m| {
                m.as_ref()
                    .map(|m| m.0.selection.exchange.clone())
                    .unwrap_or_default()
            })
        });
        let highlighted = move || highlight.get() == at.index();
        let label = move || {
            market.with(|m| {
                m.as_ref()
                    .map(|m| {
                        let s = &m.0.selection;
                        format!(
                            "{} {} {}",
                            exchanges::display_name(&s.exchange),
                            s.symbol,
                            m.0.tags
                        )
                    })
                    .unwrap_or_default()
            })
        };
        let last = move || at.index() + 1 == shown.with(|s| s.0.len());
        let kind = move || {
            market.with(|m| {
                m.as_ref()
                    .map(|m| m.0.selection.kind.label().to_owned())
                    .unwrap_or_default()
            })
        };
        let detail = move || market.with(|m| m.as_ref().and_then(|m| m.0.detail.clone()));
        view()
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(move || {
                if last() {
                    theme::BORDER.opacity(0.0).get()
                } else {
                    theme::BORDER.get()
                }
            })
            .bg(move || {
                if highlighted() || is_current.get() {
                    theme::SURFACE.get()
                } else {
                    theme::PANEL.get()
                }
            })
            .role(AccessibilityRole::Option)
            .label(label)
            .accessibility_state(move || AccessibilityState {
                selected: is_current.get(),
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
                    .w(px(CURRENT_EDGE))
                    .bg(theme::ACCENT)
                    .style_when(move || !is_current.get(), |s| s.hidden()),
            )
            .child(
                text(field(|m| m.selection.symbol.clone()))
                    .flex_none()
                    .min_w(px(80.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::TEXT),
            )
            // The rest of the row's width, named only under "All".
            .child(
                text(move || {
                    if names.get() {
                        exchanges::display_name(&exchange.get()).to_owned()
                    } else {
                        String::new()
                    }
                })
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(11.0))
                .text_color(theme::TEXT_MUTED),
            )
            .child(market_tags(kind, detail))
    }
}
