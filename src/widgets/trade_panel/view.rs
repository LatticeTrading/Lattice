//! The Trade Panel's view. Blocks edge to edge, top to bottom:
//!
//! - while there is nothing to trade for: one bordered piece in the middle
//!   of the pane, the rows that connect a wallet and then the rows that
//!   enable its exchange ([`access`](crate::widgets::common::access), the
//!   wallet panel's own);
//! - the account's bar: its exchange and address, then what it has to
//!   trade with (its equity, what is free of it, and how much of its
//!   margin its positions need), each a cell with a left edge; the last
//!   hides in a pane [`NARROW`] wide or less;
//! - the ticket, a row per choice: the market (a field, with its mid price
//!   at the far edge), how the account trades it (its leverage, typed, the
//!   margin mode, picked, and the cell that sets both), Buy and Sell as two
//!   halves, Limit and Market as two halves, the price (with a cell that
//!   fills in the mid), a limit order's time in force, the size, the take
//!   profit and the stop loss placed with it (each with what it would make
//!   or lose at its far edge; left out of an order that only reduces), and
//!   whether it only reduces;
//! - what is to be said: that the exchange lists no such market, what is
//!   missing of the order, what the last order or cancel came to;
//! - the cell that sends the order, which says what it sends in the side's
//!   color. With orders confirmed (the settings' default) the first press
//!   turns it into the question, with × beside it; the second sends;
//! - the position the account holds in the market, when it holds one: a
//!   line (which way, how much, its entry, what it has made), then the
//!   take profit and the stop loss to place for the whole of it as two
//!   fields with the cell that places them;
//! - the open orders, a line each under a title that says how many: side,
//!   size, market and price (the one it waits for, for one that waits),
//!   what kind it is with the exits placed with it, and × which cancels it.
//!
//! The ticket and the open orders scroll together under the bar when the
//! pane is shorter than they are.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{
    session::{
        Listing, Said, TradeSession, open_order_line, order_amount, order_line, order_price,
        request_of, typed,
    },
    settings::TradePanelSettings,
};
use crate::{
    data::{
        Wallet,
        portfolio::{Direction, MarginAccount, MarginMode, Position, short_address},
        trading::{OpenOrder, Side, TimeInForce},
        wallet::Phase,
    },
    exchanges, format,
    theme::{self, Role},
    ui::{self, Tone},
    widgets::{
        TabContext,
        common::{
            LogoStyle,
            access::{Voice, stacked, strip, venue_rows, wallet_rows},
            accounts::{NONE, Standing, Tone as Made, shown},
            exchange_logo, tag_text,
        },
    },
};

/// What its test ids start with.
const ID: &str = "trade-panel";
/// The account's bar.
const BAR_HEIGHT: f32 = 40.0;
/// An open order's line, and the orders' title with the line under it.
const LINE_HEIGHT: f32 = 28.0;
const TITLE_HEIGHT: f32 = 29.0;
/// A cell with a fixed width at a row's far edge.
const TRAILING: f32 = 60.0;
/// The widest the piece is while there is nothing to trade for.
const FORM_WIDTH: f32 = 360.0;
/// At or under this width the bar leaves out its last figure.
const NARROW: f32 = 420.0;
/// The narrowest the margin mode's select is: `Isolated` and its arrow.
const MODE_WIDTH: f32 = 92.0;
/// How the account's bar draws its exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// The color of a side: the book's own.
fn side_color(side: Side) -> Role {
    match side {
        Side::Buy => theme::BID,
        Side::Sell => theme::ASK,
    }
}

/// The Trade Panel of `session`, for the market its tab's settings name.
pub(super) fn trade_view(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> View {
    let trader = session.trader;
    let ready = computed(move || trader.with(Option::is_some));
    let gate = {
        let session = session.clone();
        dynamic(move || (!ready.get()).then(|| ask(&session, &session.wallet)))
    };
    let width = signal(0.0_f32);
    let narrow = computed(move || width.get() <= NARROW);
    let body = view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .style_when(move || !ready.get(), |s| s.hidden())
        .child(account_bar(session, narrow))
        .child(report_strip(session))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .relative()
                .child(
                    scroll().absolute().inset_0().child(
                        view()
                            .flex_col()
                            .child(ticket(session, tab))
                            .child(position_block(session))
                            .child(open_orders(session)),
                    ),
                ),
        );
    view()
        .test_id(ID)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .text_size(px(12.0))
        .on_layout(move |bounds: &Bounds<Points>| width.set(bounds.size.width.0))
        .child(gate)
        .child(body)
}

/// What the panel shows while there is nothing to trade for: the rows that
/// connect a wallet, then the rows that enable the exchange for its
/// account, one bordered piece in the middle (tracked: made again as the
/// connection comes along).
fn ask(session: &TradeSession, wallet: &Wallet) -> View {
    let state = wallet.with(Clone::clone);
    let venue = session.venue.get();
    let rows = match &state.phase {
        Phase::Connected(account) => match session.trading.venue(&venue) {
            Some(venue) => venue_rows(&session.trading, venue, &account.wallet, wallet.reach()),
            None => vec![strip(
                format!(
                    "{} is not available for trading in this version of Lattice.",
                    exchanges::display_name(&venue)
                ),
                Voice::Waiting,
            )],
        },
        _ => wallet_rows(wallet, &state),
    };
    view()
        .test_id(&format!("{ID}-ask"))
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.0))
        .child(
            stacked(rows)
                .w_full()
                .max_w(px(FORM_WIDTH))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER),
        )
}

/// How much of the cross margin the positions need to stay open, as the
/// bar says it; a dash while the exchange reports neither.
fn margin_ratio(margin: &MarginAccount) -> String {
    match (margin.maintenance_margin, margin.cross_equity) {
        (Some(needed), Some(equity)) if equity > 0.0 => {
            format!("{}%", format::grouped(needed / equity * 100.0, 1))
        }
        _ => NONE.to_owned(),
    }
}

/// The account's bar: whose orders these are, and what it has to trade
/// with.
fn account_bar(session: &Rc<TradeSession>, narrow: Memo<bool>) -> View {
    let (venue, venue_name, account) = (session.venue, session.venue_name, session.trader);
    let address = move || {
        account.with(|account| {
            account
                .as_ref()
                .map_or_else(String::new, |account| account.address.clone())
        })
    };
    let who = view()
        .h_full()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .px(px(8.0))
        .text_size(px(11.0))
        .test_id(&format!("{ID}-account"))
        .tooltip(ui::hint(address))
        .child(dynamic(move || exchange_logo(&venue.get(), LOGO)))
        .child(
            text(move || venue_name.get())
                .flex_none()
                .number_of_lines(1),
        )
        .child(
            text(move || short_address(&address()))
                .min_w(px(0.0))
                .truncate()
                .tabular_nums()
                .text_color(theme::TEXT_MUTED),
        );
    let figure = |name: &'static str, id: &'static str, read: fn(&MarginAccount) -> String| {
        let session = session.clone();
        view()
            .h_full()
            .flex_none()
            .flex_col()
            .justify_center()
            .items_end()
            .px(px(8.0))
            .border_l_1()
            .border_color(theme::BORDER)
            .test_id(&format!("{ID}-{id}"))
            .child(tag_text(text(name)).text_color(theme::TEXT_FAINT))
            .child(
                text(move || {
                    session
                        .standing()
                        .map_or_else(|| NONE.to_owned(), |margin| read(&margin))
                })
                .number_of_lines(1)
                .tabular_nums()
                .font_weight(FontWeight::MEDIUM),
            )
    };
    view()
        .h(px(BAR_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(who)
        .child(figure("Equity", "equity", |margin| {
            shown(margin.equity, format::usd)
        }))
        .child(figure("Available", "available", |margin| {
            shown(margin.withdrawable, format::usd)
        }))
        .child(
            figure("Margin ratio", "margin-ratio", margin_ratio)
                .style_when(move || narrow.get(), |s| s.hidden()),
        )
}

/// A warning under the bar while the account's figures could not be read
/// or are not up to date; nothing while all is well.
fn report_strip(session: &Rc<TradeSession>) -> View {
    let standing = session.margin.standing;
    let said = computed(move || match standing.get() {
        Standing::Failed(why) => Some(format!("Could not load the account: {why}")),
        Standing::Stale(why) => Some(format!("The account is not up to date: {why}")),
        _ => None,
    });
    view()
        .flex_none()
        .border_b_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-account-problem"))
        .style_when(move || said.with(Option::is_none), |s| s.hidden())
        .child(dynamic(move || {
            said.get().map(|said| strip(said, Voice::Waiting))
        }))
}

/// A row of the ticket after its first: `name` in the label column, and
/// the stack's line on its top edge (the row is a point taller for it, so
/// a field in it keeps its height).
fn row(name: &'static str) -> View {
    ui::label_column_row(name)
        .h(ui::ROW_HEIGHT + px(1.0))
        .border_t_1()
        .border_color(theme::BORDER)
}

/// One of a row's equal halves, which is `color` on a tint of it while
/// `selected`.
fn half(
    label: &'static str,
    name: &'static str,
    color: Role,
    selected: impl Fn() -> bool + Copy + 'static,
) -> View {
    ui::button(label, Tone::Normal)
        .flex_1()
        .justify_center()
        .label(name)
        .accessibility_state(move || AccessibilityState {
            selected: selected(),
            ..Default::default()
        })
        .text_color(move || {
            if selected() {
                color.get()
            } else {
                theme::TEXT_MUTED.get()
            }
        })
        .style_when(selected, move |s| s.bg(color.opacity(0.10)))
}

/// A cell at a row's far edge that only says something.
fn trailing(content: impl Fn() -> String + 'static) -> View {
    view()
        .h_full()
        .flex_none()
        .min_w(px(TRAILING))
        .flex()
        .items_center()
        .justify_end()
        .px(px(10.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .child(text(content).number_of_lines(1).tabular_nums())
}

/// The ticket: the order as it is written, and the cell that sends it.
fn ticket(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> View {
    let entry = session.ticket;
    let (symbol, listing, mid) = (session.symbol, session.listing, session.mid);
    let saved = tab.settings;

    // The market: what is typed, until it is committed to the settings.
    let draft = signal(symbol.get_untracked());
    effect(move || draft.set(symbol.get()));
    let commit: Rc<dyn Fn()> = {
        let tab = tab.clone();
        Rc::new(move || {
            let typed = draft.get_untracked();
            if typed.trim().is_empty() {
                draft.set(symbol.get_untracked());
            } else {
                tab.save(&saved.get_untracked().with_instrument(&typed));
                // As saved (trimmed), also when that is what it was.
                draft.set(saved.with_untracked(|saved| saved.instrument().to_owned()));
            }
        })
    };
    let (submit, blur) = (commit.clone(), commit);
    let market = ui::label_column_row("Market")
        .child(
            ui::text_field(draft)
                .flex_1()
                .placeholder("BTC")
                .label("Market")
                .test_id(&format!("{ID}-market"))
                .on_submit(move |_: &str| submit())
                .on_blur(move || blur()),
        )
        .child(
            trailing(move || {
                mid.get()
                    .map_or_else(|| NONE.to_owned(), format::price_text)
            })
            .test_id(&format!("{ID}-mid"))
            .tooltip(ui::hint(|| "Mid price".to_owned())),
        );

    let venue = session.venue_name;
    let unlisted = computed(move || {
        let venue = venue.get();
        match listing.get() {
            Listing::Missing => Some(format!("{venue} lists no {} market.", symbol.get())),
            Listing::Delisted(listed) => Some(format!("{listed} is delisted on {venue}.")),
            Listing::Unknown | Listing::Listed(_) => None,
        }
    });
    let unlisted_strip = view()
        .border_t_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-unlisted"))
        .style_when(move || unlisted.with(Option::is_none), |s| s.hidden())
        .child(dynamic(move || {
            unlisted.get().map(|said| strip(said, Voice::Waiting))
        }));

    // What an exit would make (or lose) of the order's price: of the one
    // typed, or of the market's for a market order.
    let moved = move |exit: Signal<String>| {
        move || {
            let from = if entry.market.get() {
                mid.get()
            } else {
                typed(&entry.price.get())
            };
            let (from, exit) = (from.filter(|from| *from > 0.0)?, typed(&exit.get())?);
            Some(match entry.side.get() {
                Side::Buy => (exit - from) / from,
                Side::Sell => (from - exit) / from,
            })
        }
    };
    let exits_left_out = move || entry.reduce_only.get();
    let take = exit_row(
        "TP",
        TAKE,
        "take-profit",
        entry.take_profit,
        moved(entry.take_profit),
    )
    .style_when(exits_left_out, |s| s.hidden());
    let stop = exit_row(
        "SL",
        STOP,
        "stop-loss",
        entry.stop_loss,
        moved(entry.stop_loss),
    )
    .style_when(exits_left_out, |s| s.hidden());

    let sides = view()
        .h(ui::ROW_HEIGHT + px(1.0))
        .w_full()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .children(Side::ALL.into_iter().enumerate().map(|(at, side)| {
            let cell = half(side.label(), side.label(), side_color(side), move || {
                entry.side.get() == side
            })
            .test_id(&format!("{ID}-side-{}", side.id()))
            .on_press(move || entry.side.set(side));
            if at == 0 {
                cell
            } else {
                cell.border_l_1().border_color(theme::BORDER)
            }
        }));

    let kind = |label: &'static str, name: &'static str, market: bool| {
        half(label, name, theme::ACCENT, move || {
            entry.market.get() == market
        })
        .border_l_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-type-{}", label.to_ascii_lowercase()))
        .on_press(move || entry.market.set(market))
    };
    let kinds = row("Type")
        .child(kind("Limit", "Limit order", false))
        .child(kind("Market", "Market order", true));

    let price = row("Price")
        .style_when(move || entry.market.get(), |s| s.hidden())
        .child(
            ui::text_field(entry.price)
                .flex_1()
                .placeholder("Limit price")
                .label("Price")
                .keyboard_type(KeyboardType::Decimal)
                .test_id(&format!("{ID}-price")),
        )
        .child(
            ui::button("Mid", Tone::Normal)
                .w(px(TRAILING))
                .flex_none()
                .justify_center()
                .border_l_1()
                .border_color(theme::BORDER)
                .label("Use the mid price")
                .test_id(&format!("{ID}-use-mid"))
                .disabled(move || mid.get().is_none())
                .on_press({
                    let session = session.clone();
                    move || session.price_at_mid()
                }),
        );

    let stands = row("TIF")
        .style_when(move || entry.market.get(), |s| s.hidden())
        .child(ui::dropdown(
            || {
                TimeInForce::ALL
                    .into_iter()
                    .map(|tif| SelectItem::new(tif.id(), described(tif)))
                    .collect()
            },
            move || SharedString::from(entry.time_in_force.get().id()),
            move || described(entry.time_in_force.get()).to_owned(),
            move |id| {
                if let Some(tif) = TimeInForce::from_id(id) {
                    entry.time_in_force.set(tif);
                }
            },
            "How long the order stands",
        ));

    let size = row("Size")
        .child(
            ui::text_field(entry.size)
                .flex_1()
                .placeholder("Size")
                .label("Size")
                .keyboard_type(KeyboardType::Decimal)
                .test_id(&format!("{ID}-size")),
        )
        .child(trailing(move || match listing.get() {
            Listing::Listed(instrument) => instrument.symbol,
            _ => symbol.get(),
        }));

    let reduce = ui::check_row(
        "Reduce only",
        move || entry.reduce_only.get(),
        || false,
        move || entry.reduce_only.update(|only| *only = !*only),
    )
    .border_t_1()
    .border_color(theme::BORDER)
    .test_id(&format!("{ID}-reduce-only"));

    view()
        .w_full()
        .flex_col()
        .child(market)
        .child(unlisted_strip)
        .child(leverage_row(session))
        .child(sides)
        .child(kinds)
        .child(price)
        .child(stands)
        .child(size)
        .child(take)
        .child(stop)
        .child(reduce)
        .child(said_strip(session.said))
        .child(send_row(session, tab))
}

/// What an exit is called: its field's name, and what it reads while empty.
const TAKE: (&str, &str) = ("Take profit price", "Take profit (optional)");
const STOP: (&str, &str) = ("Stop loss price", "Stop loss (optional)");

/// A row of the ticket for one exit placed with the order: its trigger
/// price, typed, and at the far edge what the order would have made (or
/// lost) there, once both prices are known (`moved`, a fraction).
fn exit_row(
    label: &'static str,
    (name, placeholder): (&'static str, &'static str),
    id: &str,
    price: Signal<String>,
    moved: impl Fn() -> Option<f64> + 'static,
) -> View {
    row(label)
        .child(
            ui::text_field(price)
                .flex_1()
                .placeholder(placeholder)
                .label(name)
                .keyboard_type(KeyboardType::Decimal)
                .test_id(&format!("{ID}-{id}")),
        )
        .child(
            trailing(move || moved().map_or_else(|| NONE.to_owned(), format::signed_percent))
                .test_id(&format!("{ID}-{id}-move")),
        )
}

/// What the margin modes are saved and picked by.
const fn mode_id(mode: MarginMode) -> &'static str {
    match mode {
        MarginMode::Cross => "cross",
        MarginMode::Isolated => "isolated",
    }
}

/// How the account is set to trade the market: its leverage, typed, whose
/// margin backs a position, picked, and the cell that sets both (lit while
/// they are another setting than the account's).
fn leverage_row(session: &Rc<TradeSession>) -> View {
    let (draft, mode, listing) = (session.leverage_draft, session.mode_draft, session.listing);
    let setting = session.setting;
    let listed = move || match listing.get() {
        Listing::Listed(instrument) => Some(instrument),
        _ => None,
    };
    let field = ui::text_field(draft)
        .flex_1()
        .placeholder(move || {
            let most = listed().and_then(|instrument| instrument.max_leverage);
            most.map_or_else(|| "Leverage".to_owned(), |most| format!("1 to {most}"))
        })
        .label("Leverage")
        .keyboard_type(KeyboardType::Number)
        .test_id(&format!("{ID}-leverage"))
        .on_submit({
            let session = session.clone();
            move |_: &str| session.set_leverage()
        });
    let trigger = view()
        .h_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .child(text(move || mode.get().label().to_owned()).number_of_lines(1))
        .child(text("▾").text_color(theme::TEXT_FAINT));
    let modes = ui::select_from(
        move || {
            // A market traded with a position's own margin only has no
            // other mode to pick.
            let only = listed().is_some_and(|instrument| instrument.isolated_only);
            [MarginMode::Cross, MarginMode::Isolated]
                .into_iter()
                .filter(|mode| !only || *mode == MarginMode::Isolated)
                .map(|mode| SelectItem::new(mode_id(mode), mode.label()))
                .collect()
        },
        move || SharedString::from(mode_id(mode.get())),
        move |id| {
            let picked = [MarginMode::Cross, MarginMode::Isolated]
                .into_iter()
                .find(|mode| mode_id(*mode) == id);
            if let Some(picked) = picked {
                mode.set(picked);
            }
        },
        "Margin mode",
        trigger,
    )
    .flex_none()
    .min_w(px(MODE_WIDTH))
    .h_full()
    .border_l_1()
    .border_color(theme::BORDER);
    let set = {
        let (differs, sets) = (session.clone(), session.clone());
        ui::button("Set", Tone::Normal)
            .w(px(TRAILING))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Set leverage and margin mode")
            .test_id(&format!("{ID}-set-leverage"))
            .disabled(move || setting.get() || !differs.leverage_differs())
            .on_press(move || sets.set_leverage())
    };
    // No row without a setting: an exchange that has none, or a market it
    // does not list.
    let offered = session.leverage_offered;
    row("Leverage")
        .style_when(move || !offered.get(), |s| s.hidden())
        .child(field)
        .child(modes)
        .child(set)
}

/// What a time in force is called in its dropdown.
fn described(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::GoodTilCancelled => "Good til cancelled (GTC)",
        TimeInForce::ImmediateOrCancel => "Immediate or cancel (IOC)",
        TimeInForce::PostOnly => "Post only (ALO)",
    }
}

/// What the panel says of the last order or cancel; hidden with nothing
/// to say.
fn said_strip(said: Signal<Option<Said>>) -> View {
    view()
        .border_t_1()
        .border_color(theme::BORDER)
        .test_id(&format!("{ID}-said"))
        .style_when(move || said.with(Option::is_none), |s| s.hidden())
        .child(dynamic(move || {
            said.get().map(|said| strip(said.text, said.voice))
        }))
}

/// The cell that sends the ticket's order: it says what it sends, in the
/// side's color. Asked again in place while the order is armed, with ×
/// beside it to let it go.
fn send_row(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> View {
    let entry = session.ticket;
    let (symbol, listing) = (session.symbol, session.listing);
    let (armed, sending, session_mid) = (session.armed, session.sending, session.mid);
    let saved = tab.settings;
    // The instrument as the exchange writes it, once it is known.
    let instrument = move || match listing.get() {
        Listing::Listed(instrument) => instrument.symbol,
        _ => symbol.get(),
    };
    let says = move || {
        if sending.get() {
            return "Sending…".to_owned();
        }
        if let Some(order) = armed.get() {
            return format!("Confirm: {}", order_line(&order));
        }
        let instrument = instrument();
        match request_of(&entry.draft(), &instrument, session_mid.get()) {
            Ok(order) => order_line(&order),
            Err(_) => format!("{} {instrument}", entry.side.get().label()),
        }
    };
    let unusable = move || sending.get() || !matches!(listing.get(), Listing::Listed(_));
    let send = {
        let session = session.clone();
        view()
            .h_full()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .px(px(10.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || side_color(entry.side.get()).get())
            .role(AccessibilityRole::Button)
            .label(says)
            .test_id(&format!("{ID}-send"))
            .disabled(unusable)
            .hover(|s| s.bg(theme::SURFACE))
            .active(|s| s.bg(theme::BORDER))
            .disabled_style(|s| s.opacity(0.5))
            .style_when(
                move || armed.with(Option::is_some),
                move |s| s.bg(side_color(entry.side.get_untracked()).opacity(0.10)),
            )
            .on_press(move || session.submit(saved.with_untracked(TradePanelSettings::confirm)))
            .child(text(says).min_w(px(0.0)).truncate())
    };
    let back = {
        let session = session.clone();
        ui::button("×", Tone::Quiet)
            .w(ui::ROW_HEIGHT)
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Do not send the order")
            .test_id(&format!("{ID}-disarm"))
            .style_when(move || armed.with(Option::is_none), |s| s.hidden())
            .on_press(move || session.disarm())
    };
    view()
        .h(ui::ROW_HEIGHT + px(1.0))
        .w_full()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .child(send)
        .child(back)
}

/// The title of a part of the panel under the ticket, with the lines that
/// part it from what is above and below it; what its far edge says is the
/// caller's to add.
fn section_title(name: &'static str) -> View {
    view()
        .h(px(TITLE_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(10.0))
        .bg(theme::SURFACE)
        .border_t_1()
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            text(name)
                .number_of_lines(1)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .letter_spacing(px(0.8))
                .uppercase()
                .text_color(theme::TEXT_FAINT),
        )
}

/// What a position has made, with its return on what it holds:
/// `+$10.00 (+16.67%)`; a dash when the exchange reports neither.
fn made(position: &Position) -> String {
    match (position.unrealized_pnl, position.return_on_equity) {
        (Some(pnl), Some(return_on_equity)) => format!(
            "{} ({})",
            format::signed_usd(pnl),
            format::signed_percent(return_on_equity)
        ),
        (Some(pnl), None) => format::signed_usd(pnl),
        (None, _) => NONE.to_owned(),
    }
}

/// The position's line: which way it faces, how much of what, where it
/// was entered, and at the far edge what it has made.
fn position_line(position: &Position) -> View {
    let (facing, color) = match position.direction {
        Direction::Long => ("Long", theme::POSITIVE),
        Direction::Short => ("Short", theme::NEGATIVE),
    };
    let tone = position.unrealized_pnl.map(Made::of);
    view()
        .h(px(LINE_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .child(tag_text(text(facing)).flex_none().text_color(color))
        .child(
            text(format!(
                "{} {}",
                format::quantity(position.size),
                position.instrument
            ))
            .flex_none()
            .number_of_lines(1)
            .tabular_nums(),
        )
        .child(
            text(format!(
                "at {}",
                shown(position.entry_price, format::price_text)
            ))
            .flex_none()
            .number_of_lines(1)
            .tabular_nums()
            .text_color(theme::TEXT_MUTED),
        )
        .child(
            text(made(position))
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_right()
                .tabular_nums()
                .text_color(move || Made::color(tone)),
        )
}

/// The position the account holds in the market, while it holds one: its
/// line, then the exits to place for the whole of it (a take profit and a
/// stop loss, either or both) and the cell that places them. The exits it
/// has are among the open orders below, each cancelled as one.
fn position_block(session: &Rc<TradeSession>) -> View {
    let (position, protecting) = (session.position, session.protecting);
    let line = dynamic(move || position.get().map(|position| position_line(&position)));
    let field = |price: Signal<String>, name: &'static str, id: &str| {
        let session = session.clone();
        ui::text_field(price)
            .flex_1()
            .placeholder(name)
            .label(format!("{name} for the position"))
            .keyboard_type(KeyboardType::Decimal)
            .test_id(&format!("{ID}-position-{id}"))
            .on_submit(move |_: &str| session.protect())
    };
    let set = {
        let session = session.clone();
        ui::button("Set", Tone::Normal)
            .w(px(TRAILING))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label("Place the position's exits")
            .test_id(&format!("{ID}-position-set"))
            .disabled(move || protecting.get())
            .on_press(move || session.protect())
    };
    let exits = ui::label_column_row("Exits")
        .child(field(session.protect_take, "Take profit", "take-profit"))
        .child(
            field(session.protect_stop, "Stop loss", "stop-loss")
                .border_l_1()
                .border_color(theme::BORDER),
        )
        .child(set);
    view()
        .w_full()
        .flex_col()
        .test_id(&format!("{ID}-position"))
        .style_when(move || position.with(Option::is_none), |s| s.hidden())
        .child(section_title("Position"))
        .child(line)
        .child(exits)
}

/// The account's open orders: a title that says how many, then a line
/// each.
fn open_orders(session: &Rc<TradeSession>) -> View {
    let orders = {
        let session = session.clone();
        computed(move || {
            session
                .open_orders()
                .map(|orders| orders.0.clone())
                .unwrap_or_default()
        })
    };
    let standing = session.orders.standing;
    let count = move || match standing.get() {
        Standing::Loading | Standing::Unset => "…".to_owned(),
        Standing::Failed(_) | Standing::Unavailable(_) => NONE.to_owned(),
        _ => orders.with(Vec::len).to_string(),
    };
    let title = section_title("Open orders").child(
        text(count)
            .tabular_nums()
            .text_size(px(10.0))
            .text_color(theme::TEXT_FAINT),
    );
    let failed = computed(move || match standing.get() {
        Standing::Failed(why) => Some(format!("Could not load the open orders: {why}")),
        Standing::Stale(why) => Some(format!("The open orders are not up to date: {why}")),
        _ => None,
    });
    let failed_strip = view()
        .border_b_1()
        .border_color(theme::BORDER)
        .style_when(move || failed.with(Option::is_none), |s| s.hidden())
        .child(dynamic(move || {
            failed.get().map(|said| strip(said, Voice::Waiting))
        }));
    let none = view()
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .test_id(&format!("{ID}-no-orders"))
        .style_when(
            move || !matches!(standing.get(), Standing::Ready) || orders.with(|o| !o.is_empty()),
            |s| s.hidden(),
        )
        .child(text("No open orders"));
    let lines = {
        let session = session.clone();
        each(
            move || orders.get(),
            |order: &OpenOrder| order.id.clone(),
            move |order, _| order_row(&session, order),
        )
    };
    view()
        .w_full()
        .flex_col()
        .test_id(&format!("{ID}-orders"))
        .child(title)
        .child(failed_strip)
        .child(none)
        .child(lines)
}

/// What an open order is besides its side, size and price: its kind, the
/// exits placed with it, and that it only reduces (which an exit always
/// does, and need not say).
fn detail(order: &OpenOrder) -> String {
    let mut detail = order.kind.clone();
    if let Some(price) = order.exits.take_profit {
        detail.push_str(&format!(" · TP {}", format::price_text(price)));
    }
    if let Some(price) = order.exits.stop_loss {
        detail.push_str(&format!(" · SL {}", format::price_text(price)));
    }
    if order.reduce_only && order.trigger_price.is_none() {
        detail.push_str(", reduce only");
    }
    detail
}

/// One open order's line: its side, its size and market, its price (the
/// one it waits for, for one that waits), what kind it is, and × which
/// cancels it.
fn order_row(session: &Rc<TradeSession>, order: ReadSignal<OpenOrder>) -> View {
    // An order keeps its id (its key) and its side.
    let (id, side) = order.with_untracked(|order| (order.id.clone(), order.side));
    let cancelling = session.cancelling;
    let waiting = {
        let id = id.clone();
        move || cancelling.with(|cancelling| cancelling.contains(&id))
    };
    let cancel = {
        let session = session.clone();
        ui::button("×", Tone::Quiet)
            .w(px(LINE_HEIGHT))
            .flex_none()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label(move || format!("Cancel {}", order.with(open_order_line)))
            .test_id(&format!("{ID}-cancel-{id}"))
            .disabled(waiting)
            .on_press(move || session.cancel(&order.get_untracked()))
    };
    view()
        .h(px(LINE_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .test_id(&format!("{ID}-order-{id}"))
        .child(
            view()
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .pl(px(10.0))
                .pr(px(8.0))
                .child(
                    tag_text(text(side.label()))
                        .flex_none()
                        .text_color(side_color(side)),
                )
                .child(
                    text(move || order.with(order_amount))
                        .flex_none()
                        .number_of_lines(1)
                        .tabular_nums(),
                )
                .child(
                    text(move || order.with(|order| shown(order_price(order), format::price_text)))
                        .flex_none()
                        .number_of_lines(1)
                        .tabular_nums()
                        .text_color(theme::TEXT_MUTED),
                )
                .child(
                    text(move || order.with(detail))
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_right()
                        .text_color(theme::TEXT_FAINT),
                ),
        )
        .child(cancel)
}
