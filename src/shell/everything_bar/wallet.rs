//! The wallet: the bar's button and the panel that hangs from it.
//!
//! - The button stands between the wordmark and the Layout Manager's. It
//!   reads "Connect Wallet" until one is connected ("Connect" in a window
//!   too narrow for that beside the centered search, [`ROOMY`]), then a
//!   green dot and the account's address in short; its text is the accent
//!   color while its panel is open, as every cell of the bar that opens
//!   something.
//! - The panel is the whole connection, a stack of rows for where it has
//!   come ([`Phase`]): the button that starts it; what is being waited for
//!   (on desktop, the page in the user's browser, with its link to open
//!   again or copy); the wallets to choose from; the connected account
//!   with the Hyperliquid test and its two results; Disconnect.
//!
//! Everything it shows is the wallet service's state
//! ([`Wallet`](crate::data::Wallet)); nothing of the connection lives
//! here, so closing the panel changes nothing of it.

use nsh_core::{LineHeight, prelude::*};

use crate::{
    data::{
        Wallet,
        portfolio::short_address,
        wallet::{Account, Check, Outcome, Phase, Reach, WalletInfo, WalletState},
    },
    theme,
    ui::{self, Side, Tone},
};

/// The connected dot's diameter.
const DOT: Points = px(6.0);
/// A wallet's line in the list to choose from.
const WALLET_ROW: Points = px(30.0);
/// The window width from which the cell says "Connect Wallet" in full. In a
/// narrower one the bar's left group, with the default two pages, would be
/// wider than its side of the bar and push the search off the middle.
const ROOMY: f32 = 1360.0;

/// The bar's wallet: the service, whether the panel is open and what it
/// hangs from.
#[derive(Clone)]
pub(super) struct WalletCell {
    wallet: Wallet,
    open: Signal<bool>,
    anchor: ElementRef,
}

/// What a strip of the panel says, and so how it is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Voice {
    /// Something to know.
    Plain,
    /// Something the user is waited for, or should look at.
    Waiting,
    Failed,
    Passed,
}

impl WalletCell {
    pub(super) fn new(wallet: Wallet) -> Self {
        Self {
            wallet,
            open: signal(false),
            anchor: element_ref(),
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The bar's cell.
    pub(super) fn button(&self) -> View {
        let open = self.open;
        let roomy = computed(|| window().width.0 >= ROOMY);
        let connected = {
            let wallet = self.wallet.clone();
            move || wallet.with(|state| matches!(state.phase, Phase::Connected(_)))
        };
        let says = {
            let wallet = self.wallet.clone();
            move || wallet.with(|state| cell_text(&state.phase, roomy.get()))
        };
        let describes = {
            let wallet = self.wallet.clone();
            move || wallet.with(|state| cell_label(&state.phase))
        };
        view()
            .element_ref(&self.anchor)
            .test_id("wallet")
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .border_l_1()
            .border_color(theme::BORDER)
            .text_size(px(12.0))
            .text_color(move || {
                if open.get() {
                    theme::ACCENT.get()
                } else {
                    theme::TEXT_MUTED.get()
                }
            })
            .role(AccessibilityRole::Button)
            .label(describes)
            .accessibility_state(move || AccessibilityState {
                expanded: Some(open.get()),
                has_popup: true,
                ..Default::default()
            })
            .on_press(move || open.update(|open| *open = !*open))
            .child(
                view()
                    .size(DOT)
                    .flex_none()
                    .rounded_full()
                    .bg(theme::SUCCESS)
                    .style_when(move || !connected(), |style| style.hidden()),
            )
            .child(text(says).number_of_lines(1))
    }

    /// The panel, below the button.
    pub(super) fn panel(&self) -> Overlay {
        let open = self.open;
        let this = self.clone();
        let body = dynamic(move || {
            let state = this.wallet.with(Clone::clone);
            let rows = this.rows(&state);
            view()
                .test_id("wallet-panel")
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    if index == 0 {
                        row
                    } else {
                        row.border_t_1().border_color(theme::BORDER)
                    }
                }))
        });
        ui::floating_panel(
            &self.anchor,
            Side::Left,
            // As wide as the Layout Manager, which opens beside it.
            super::PANEL_WIDTH,
            "Wallet",
            move || open.set(false),
            body,
        )
    }

    /// The panel's rows for `state`, top to bottom. Each after the first
    /// gets the stack's line on its top edge.
    fn rows(&self, state: &WalletState) -> Vec<View> {
        let reach = self.wallet.reach();
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
                rows.push(self.action(label, Tone::Normal, Wallet::connect));
                rows.extend(note.map(|note| strip(note.to_owned(), Voice::Plain)));
            }
            Phase::Finding => {
                let waiting = match reach {
                    Reach::Page => "Looking for wallets…",
                    Reach::Browser => "Waiting for the wallet page in your browser…",
                };
                rows.push(strip(waiting.to_owned(), Voice::Plain));
                rows.extend(failure(Voice::Waiting));
                rows.extend(self.page_row(state));
                rows.push(self.action("Cancel", Tone::Quiet, Wallet::disconnect));
            }
            Phase::Choosing(wallets) => {
                rows.extend(failure(Voice::Failed));
                rows.extend(wallets.iter().map(|wallet| self.wallet_row(wallet)));
                rows.push(self.action("Cancel", Tone::Quiet, Wallet::disconnect));
            }
            Phase::Approving(wallet) => {
                let asks = match reach {
                    Reach::Page => format!("Approve the connection in {}.", wallet.name),
                    Reach::Browser => format!(
                        "Approve the connection in {}, in your browser.",
                        wallet.name
                    ),
                };
                rows.push(strip(asks, Voice::Waiting));
                rows.extend(self.page_row(state));
                rows.push(self.action("Cancel", Tone::Quiet, Wallet::disconnect));
            }
            Phase::Connected(account) => {
                rows.extend(failure(Voice::Failed));
                rows.push(fact("Wallet", account.wallet.name.clone()));
                rows.push(address_row(account));
                rows.push(fact("Network", network(account.chain_id)));
                rows.push(
                    self.action("Test Hyperliquid", Tone::Normal, Wallet::check_hyperliquid)
                        .disabled(state.check.running()),
                );
                rows.extend(check_strips(&state.check, account, reach));
                rows.push(self.action("Disconnect", Tone::Quiet, Wallet::disconnect));
            }
        }
        rows
    }

    /// A row that is one action of the wallet's.
    fn action(&self, label: &'static str, tone: Tone, act: fn(&Wallet)) -> View {
        let wallet = self.wallet.clone();
        ui::button(label, tone)
            .h(ui::ROW_HEIGHT)
            .w_full()
            .label(label)
            .on_press(move || act(&wallet))
    }

    /// The bridge page's row, where the wallets are reached through one:
    /// two equal halves, to open it again and to copy its link (for the
    /// browser the wallet is in, when that is not the one that opened).
    fn page_row(&self, state: &WalletState) -> Option<View> {
        let page = state.page.clone()?;
        let wallet = self.wallet.clone();
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
    fn wallet_row(&self, found: &WalletInfo) -> View {
        let (wallet, id) = (self.wallet.clone(), found.id.clone());
        ui::button(found.name.clone(), Tone::Normal)
            .h(WALLET_ROW)
            .w_full()
            .label(format!("Connect {}", found.name))
            .on_press(move || wallet.choose(&id))
            .child(view().flex_1())
            .child(text("›").text_color(theme::TEXT_FAINT))
    }
}

/// What the bar's cell reads; `roomy` is whether there is room to say it
/// in full.
fn cell_text(phase: &Phase, roomy: bool) -> String {
    match phase {
        Phase::Disconnected if roomy => "Connect Wallet".to_owned(),
        Phase::Disconnected => "Connect".to_owned(),
        Phase::Finding | Phase::Choosing(_) | Phase::Approving(_) => "Connecting…".to_owned(),
        Phase::Connected(account) => short_address(&account.address_text()),
    }
}

/// What the bar's cell is called.
fn cell_label(phase: &Phase) -> String {
    match phase {
        Phase::Disconnected => "Wallet: not connected".to_owned(),
        Phase::Finding | Phase::Choosing(_) | Phase::Approving(_) => {
            "Wallet: connecting".to_owned()
        }
        Phase::Connected(account) => format!("Wallet: {}", account.address_text()),
    }
}

/// A fact of the connection: its name in the label column, then the fact.
fn fact(name: &'static str, value: String) -> View {
    ui::label_column_row(name).child(
        text(value)
            .flex_1()
            .min_w(px(0.0))
            .pr(px(10.0))
            .number_of_lines(1),
    )
}

/// The account's address in short, whole in its tooltip, with the cell
/// that copies it.
fn address_row(account: &Account) -> View {
    let address = account.address_text();
    let whole = address.clone();
    ui::label_column_row("Account")
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .tooltip(ui::hint(move || whole.clone()))
                .child(text(short_address(&address)).number_of_lines(1)),
        )
        .child(
            ui::button("Copy", Tone::Normal)
                .w(px(60.0))
                .flex_none()
                .justify_center()
                .border_l_1()
                .border_color(theme::BORDER)
                .label("Copy the account's address")
                .on_press(move || set_clipboard(address.clone())),
        )
}

/// The strips of the Hyperliquid test, for the checks made or being made.
fn check_strips(check: &Check, account: &Account, reach: Reach) -> Vec<View> {
    let signing = match reach {
        Reach::Page => format!(
            "Sign the test message in {}. It authorizes nothing.",
            account.wallet.name
        ),
        Reach::Browser => format!(
            "Sign the test message in {}, in your browser. It authorizes nothing.",
            account.wallet.name
        ),
    };
    let said = |what: &str, outcome: &Outcome, running: String| match outcome {
        Outcome::Idle => None,
        Outcome::Running => Some(strip(running, Voice::Waiting)),
        Outcome::Passed(found) => Some(strip(format!("{what}: {found}"), Voice::Passed)),
        Outcome::Failed(wrong) => Some(strip(format!("{what}: {wrong}"), Voice::Failed)),
    };
    let asking = "Asking Hyperliquid about this account…".to_owned();
    said("Hyperliquid", &check.account, asking)
        .into_iter()
        .chain(said("Signature", &check.signature, signing))
        .collect()
}

/// A strip of the panel: `said`, in the color of its `voice`, with the
/// thick left edge and the tint when it is more than something to know.
fn strip(said: String, voice: Voice) -> View {
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
        .child(text(said).line_height(LineHeight::Multiple(1.5)));
    if voice == Voice::Plain {
        strip
    } else {
        strip
            .border_l(px(3.0))
            .border_l_color(role)
            .bg(role.opacity(0.1))
    }
}

/// What the chain `chain_id` is called: its name when it is one a wallet
/// is usually on, and its number.
fn network(chain_id: u64) -> String {
    let name = match chain_id {
        1 => "Ethereum",
        10 => "OP Mainnet",
        56 => "BNB Chain",
        137 => "Polygon",
        999 => "HyperEVM",
        8453 => "Base",
        42_161 => "Arbitrum One",
        43_114 => "Avalanche",
        421_614 => "Arbitrum Sepolia",
        11_155_111 => "Sepolia",
        _ => return format!("Chain {chain_id}"),
    };
    format!("{name} ({chain_id})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_a_wallet_is_usually_on_are_named() {
        assert_eq!(network(42_161), "Arbitrum One (42161)");
        assert_eq!(network(1), "Ethereum (1)");
        assert_eq!(network(31_337), "Chain 31337");
    }
}
