//! The wallet: the bar's button and the panel that hangs from it.
//!
//! - The button stands between the wordmark and the Layout Manager's. It
//!   reads "Connect Wallet" until one is connected ("Connect" in a window
//!   too narrow for that beside the centered search, [`ROOMY`]), then a
//!   green dot and the account's address in short; its text is the accent
//!   color while its panel is open, as every cell of the bar that opens
//!   something.
//! - The panel is the whole connection, a stack of rows for where it has
//!   come ([`Phase`]): the rows that make it
//!   ([`wallet_rows`], which a trading widget shows too); then the
//!   connected account, each exchange it can trade on with where it stands
//!   there and the cell that enables or disables it ([`venue_rows`]), and
//!   Disconnect.
//!
//! Everything it shows is the services' state
//! ([`Wallet`](crate::data::Wallet), [`Trading`](crate::data::Trading));
//! nothing of the connection lives here, so closing the panel changes
//! nothing of it.

use nsh_core::prelude::*;

use crate::{
    data::{
        Trading, Wallet,
        portfolio::short_address,
        wallet::{Account, Phase, WalletState},
    },
    theme,
    ui::{self, Side, Tone},
    widgets::common::access::{Voice, stacked, strip, venue_rows, wallet_rows},
};

/// The connected dot's diameter.
const DOT: Points = px(6.0);
/// The window width from which the cell says "Connect Wallet" in full. In a
/// narrower one the bar's left group, with the default two pages, would be
/// wider than its side of the bar and push the search off the middle.
const ROOMY: f32 = 1360.0;

/// The bar's wallet: the services, whether the panel is open and what it
/// hangs from.
#[derive(Clone)]
pub(super) struct WalletCell {
    wallet: Wallet,
    trading: Trading,
    open: Signal<bool>,
    anchor: ElementRef,
}

impl WalletCell {
    pub(super) fn new(wallet: Wallet, trading: Trading) -> Self {
        Self {
            wallet,
            trading,
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
            stacked(this.rows(&state)).test_id("wallet-panel")
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

    /// The panel's rows for `state`, top to bottom (tracked: they follow
    /// where the account stands with each exchange too).
    fn rows(&self, state: &WalletState) -> Vec<View> {
        let Phase::Connected(account) = &state.phase else {
            return wallet_rows(&self.wallet, state);
        };
        let mut rows = Vec::new();
        rows.extend(
            state
                .failure
                .clone()
                .map(|failure| strip(failure, Voice::Failed)),
        );
        rows.push(fact("Wallet", account.wallet.name.clone()));
        rows.push(address_row(account));
        rows.push(fact("Network", network(account.chain_id)));
        let reach = self.wallet.reach();
        for venue in self.trading.venues() {
            rows.extend(venue_rows(&self.trading, venue, &account.wallet, reach));
        }
        let wallet = self.wallet.clone();
        rows.push(
            ui::button("Disconnect", Tone::Quiet)
                .h(ui::ROW_HEIGHT)
                .w_full()
                .label("Disconnect")
                .on_press(move || wallet.disconnect()),
        );
        rows
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
                .on_press(move || drop(set_clipboard(address.clone()))),
        )
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
