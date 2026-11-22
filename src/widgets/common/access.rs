//! The rows that bring an account to trading, the same wherever they show
//! (the Everything Bar's wallet panel, and a trading widget that has
//! nothing to trade for yet):
//!
//! - [`wallet_rows`]: connecting a wallet, for where the connection has
//!   come ([`Phase`]): the button that starts it; what is being waited for
//!   (on desktop, the page in the user's browser, with its link to open
//!   again or copy); the wallets to choose from; Cancel.
//! - [`venue_rows`]: where the connected account stands with one exchange
//!   ([`Access`]): a line with the exchange, how it stands and the one
//!   thing to do about it at its far edge (Enable, Cancel, Disable); what
//!   enabling asks, before it is asked; what the wallet is waited for; why
//!   it did not come to be. While what an earlier run kept is being taken
//!   up again the line says so, and there is nothing to do but wait.
//! - [`strip`], what either says ([`Voice`]), and [`stacked`], which lays
//!   rows in one stack with one line between each; [`Said`] is what a
//!   widget that acts says of what it was asked to do, in a strip.
//!
//! Everything shown is the services' state ([`Wallet`], [`Trading`]):
//! nothing of a connection lives in a view, so closing whatever shows
//! these changes nothing of it.

use std::rc::Rc;

use nsh_core::{LineHeight, prelude::*};

use super::{LogoStyle, exchange_logo};
use crate::{
    data::{
        Trading, Wallet,
        trading::{Access, TradingVenue},
        wallet::{Phase, Reach, WalletInfo, WalletState},
    },
    theme,
    ui::{self, Tone},
};

/// A wallet's line in the list to choose from.
const WALLET_ROW: Points = px(30.0);
/// The width of an exchange's action at its row's far edge.
const ACTION_WIDTH: Points = px(60.0);
/// How an exchange's row draws its logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// What a strip says, and so how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Voice {
    /// Something to know.
    Plain,
    /// Something the user is waited for, or should look at.
    Waiting,
    Failed,
    Passed,
}

/// Something a widget says of what it was asked to do: what came of it, or
/// why nothing was done ([`strip`] shows it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Said {
    pub text: String,
    pub voice: Voice,
}

/// A strip: `said`, in the color of its `voice`, with the thick left edge
/// and the tint when it is more than something to know.
pub(crate) fn strip(said: impl Into<String>, voice: Voice) -> View {
    let role = match voice {
        Voice::Plain => theme::TEXT_MUTED,
        Voice::Waiting => theme::WARNING,
        Voice::Failed => theme::DANGER,
        Voice::Passed => theme::SUCCESS,
    };
    let strip = view()
        .w_full()
        .px(px(10.0))
        .py(px(5.0))
        .text_size(px(11.0))
        .text_color(role)
        .role(if voice == Voice::Failed {
            AccessibilityRole::Alert
        } else {
            AccessibilityRole::Text
        })
        .child(text(said.into()).line_height(LineHeight::Multiple(1.5)));
    if voice == Voice::Plain {
        strip
    } else {
        strip
            .border_l(px(3.0))
            .border_l_color(role)
            .bg(role.opacity(0.1))
    }
}

/// `rows` as one stack, top to bottom: each after the first gets the
/// stack's line on its top edge.
pub(crate) fn stacked(rows: Vec<View>) -> View {
    view()
        .flex_col()
        .children(rows.into_iter().enumerate().map(|(index, row)| {
            if index == 0 {
                row
            } else {
                row.border_t_1().border_color(theme::BORDER)
            }
        }))
}

/// A row that is one action of the wallet's.
fn action(wallet: &Wallet, label: &'static str, tone: Tone, act: fn(&Wallet)) -> View {
    let wallet = wallet.clone();
    ui::button(label, tone)
        .h(ui::ROW_HEIGHT)
        .w_full()
        .label(label)
        .on_press(move || act(&wallet))
}

/// The bridge page's row, where the wallets are reached through one: two
/// equal halves, to open it again and to copy its link (for the browser
/// the wallet is in, when that is not the one that opened).
fn page_row(wallet: &Wallet, state: &WalletState) -> Option<View> {
    let page = state.page.clone()?;
    let wallet = wallet.clone();
    let half = |label: &'static str| ui::button(label, Tone::Normal).flex_1().justify_center();
    Some(
        view()
            .h(ui::ROW_HEIGHT)
            .w_full()
            .flex_row()
            .child(
                half("Open page again")
                    .label("Open the wallet page again")
                    .on_press(move || wallet.reopen_page()),
            )
            .child(
                half("Copy link")
                    .label("Copy the wallet page's link")
                    .border_l_1()
                    .border_color(theme::BORDER)
                    .on_press(move || set_clipboard(page.clone())),
            ),
    )
}

/// A wallet to choose: its name, and what a press does at the far edge.
fn wallet_row(wallet: &Wallet, found: &WalletInfo) -> View {
    let (wallet, id) = (wallet.clone(), found.id.clone());
    ui::button(found.name.clone(), Tone::Normal)
        .h(WALLET_ROW)
        .w_full()
        .label(format!("Connect {}", found.name))
        .on_press(move || wallet.choose(&id))
        .child(view().flex_1())
        .child(text("›").text_color(theme::TEXT_FAINT))
}

/// The rows that connect a wallet, for where `state` says the connection
/// has come; none once it is made.
pub(crate) fn wallet_rows(wallet: &Wallet, state: &WalletState) -> Vec<View> {
    let reach = wallet.reach();
    let mut rows = Vec::new();
    let failure = |voice: Voice| state.failure.clone().map(|failure| strip(failure, voice));
    match &state.phase {
        Phase::Disconnected => {
            rows.extend(failure(Voice::Failed));
            let (label, note) = match reach {
                Reach::Page => ("Connect wallet", None),
                Reach::Browser => (
                    "Connect wallet in browser",
                    Some(
                        "A page opens in your browser, where your wallet is. \
                         Keep it open while connected.",
                    ),
                ),
            };
            rows.push(action(wallet, label, Tone::Normal, Wallet::connect));
            rows.extend(note.map(|note| strip(note, Voice::Plain)));
        }
        Phase::Finding => {
            let waiting = match reach {
                Reach::Page => "Looking for wallets…",
                Reach::Browser => "Waiting for the wallet page in your browser…",
            };
            rows.push(strip(waiting, Voice::Plain));
            rows.extend(failure(Voice::Waiting));
            rows.extend(page_row(wallet, state));
            rows.push(action(wallet, "Cancel", Tone::Quiet, Wallet::disconnect));
        }
        Phase::Choosing(wallets) => {
            rows.extend(failure(Voice::Failed));
            rows.extend(wallets.iter().map(|found| wallet_row(wallet, found)));
            rows.push(action(wallet, "Cancel", Tone::Quiet, Wallet::disconnect));
        }
        Phase::Approving(asked) => {
            let asks = match reach {
                Reach::Page => format!("Approve the connection in {}.", asked.name),
                Reach::Browser => {
                    format!("Approve the connection in {}, in your browser.", asked.name)
                }
            };
            rows.push(strip(asks, Voice::Waiting));
            rows.extend(page_row(wallet, state));
            rows.push(action(wallet, "Cancel", Tone::Quiet, Wallet::disconnect));
        }
        Phase::Connected(_) => {}
    }
    rows
}

/// The rows of trading on `venue` for the connected account, whose wallet
/// is `wallet` (tracked: they are made again as its access changes).
pub(crate) fn venue_rows(
    trading: &Trading,
    venue: &Rc<dyn TradingVenue>,
    wallet: &WalletInfo,
    reach: Reach,
) -> Vec<View> {
    let (id, name) = (venue.venue(), venue.name());
    let access = trading.access(id);
    let (stands, color, does, label) = match &access {
        Access::Locked | Access::Failed(_) => (
            "Trading off",
            theme::TEXT_MUTED,
            "Enable",
            format!("Enable trading on {name}"),
        ),
        Access::Resuming => (
            "Checking…",
            theme::TEXT_MUTED,
            "Enable",
            format!("Checking whether trading is still enabled on {name}"),
        ),
        Access::Enabling => (
            "Waiting for the wallet…",
            theme::WARNING,
            "Cancel",
            format!("Cancel enabling trading on {name}"),
        ),
        Access::Ready => (
            "Trading on",
            theme::SUCCESS,
            "Disable",
            format!("Disable trading on {name}"),
        ),
    };
    let enables = matches!(access, Access::Locked | Access::Failed(_));
    let trading = trading.clone();
    let row = view()
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_row()
        .text_size(px(11.0))
        .test_id(&format!("trading-{id}"))
        .child(
            view()
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .child(exchange_logo(id, LOGO))
                .child(
                    text(name)
                        .flex_none()
                        .number_of_lines(1)
                        .text_color(theme::TEXT),
                )
                .child(text(stands).min_w(px(0.0)).truncate().text_color(color)),
        )
        .child(
            ui::button(does, Tone::Normal)
                .w(ACTION_WIDTH)
                .flex_none()
                .justify_center()
                .border_l_1()
                .border_color(theme::BORDER)
                .label(label)
                // Nothing to do while the exchange is being asked.
                .disabled(access == Access::Resuming)
                .on_press(move || {
                    if enables {
                        trading.enable(id);
                    } else {
                        trading.disable(id);
                    }
                }),
        );
    let mut rows = vec![row];
    match access {
        Access::Locked => rows.push(strip(venue.enabling_note(), Voice::Plain)),
        Access::Enabling => {
            let asks = match reach {
                Reach::Page => format!("Sign the approval in {}.", wallet.name),
                Reach::Browser => {
                    format!("Sign the approval in {}, in your browser.", wallet.name)
                }
            };
            rows.push(strip(asks, Voice::Waiting));
        }
        Access::Failed(why) => {
            rows.push(strip(format!("{name}: {why}"), Voice::Failed));
            rows.push(strip(venue.enabling_note(), Voice::Plain));
        }
        Access::Resuming | Access::Ready => {}
    }
    rows
}
