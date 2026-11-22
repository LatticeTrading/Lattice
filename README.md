# Lattice

Native Rust trading terminal built with **Native Shell (NSH) 0.6.1**, a port
of the lattice-pro web app. Lattice has one shared Rust UI for native desktop
and WebAssembly web targets.

The shell has the multi-page workspace, responsive grid with movable and
resizable panes, tabs, themes, and versioned persistence (layout codes
included). Market data comes from the Ferris backend through a shared data
layer (HTTP, WebSocket, polling). Seven widgets are ported: the Chart
(candles and ticks, with drawing tools: a trend line, a Fibonacci
retracement and a measurement; indicators: an EMA, a TEMA and Bollinger
Bands; and a fast mode, which makes the newest candle from the market's
live trades), the Premium/Arb Chart (the difference between two
markets' prices, on the Chart's plot), the Order Book (one or two markets'
depth on a price ladder) and the Depth Chart (their running depth as curves
about the mid price), all four drawn on the GPU through NSH's WGSL shader
views, the Market Screener, the Trade Stream and the Funding Rates (a
watchlist of perpetuals' funding); the others still to port have empty folders
under `src/widgets/`. Two widgets show DefiLlama's data, which Ferris proxies
and caches (the app never asks DefiLlama itself): the DeFi Protocols
(protocols ranked by total value locked, searched and filtered by category)
and the Protocol TVL (one protocol's TVL over time, on the Chart's plot, with
its share by chain); each credits DefiLlama and opens its page there.
Four portfolio widgets watch accounts by their public
addresses: the Positions, the Account Summary (total value, margin health,
exposure, fees, staking and balances), the Trade History (the latest trades,
fees and closed PnL) and the Account Performance (value or profit over
time, on the Chart's plot). Each shows one account or several together, on
one exchange or on several: what they hold listed in one table, each row
marked with its account, and what they are worth and made added up.
Lists of addresses can be kept as named groups ("Groups", beside Add where
accounts are set), each address with the exchanges it is watched on, and a
whole group given to a widget at once.
An account's data is asked of its exchange directly rather than of Ferris, so
the exchange's rate limit is each user's own; Hyperliquid is the first
exchange and Lighter the second (an L1 address's accounts there together:
their margin, positions, balances, history and trades, from the requests
Lighter answers without an auth token) and TxFlow the third (an address's
perpetuals margin, positions, trades, vault shares, fee tier and its day,
week and month figures; no token balances or history chart, which TxFlow's
API does not publish),
behind an exchange-neutral portfolio layer the widgets read. The
Everything Bar's symbol search finds any
market and sends it to the widgets linked to the bar's link color (the
Chart, from its pane header's link button); its connection indicator shows
the health of the app's data sources and opens a panel of everything the
widgets stream and poll. Next to the wordmark the bar connects the user's
browser wallet (Rabby, MetaMask, any EIP-1193 wallet) through
[alloy](https://alloy.rs): the web build asks the wallets of its own page,
and the desktop build opens a page it serves on this machine in the user's
browser, which carries its requests to the wallet there. The wallet's key is
never in the app. The connected account can trade: the Trade Panel places
and cancels orders for it (limit and market, each with a take profit and a
stop loss if wanted, on Hyperliquid's perpetuals first), sets its leverage
and margin mode per market, and places exits for a position it holds; the
Positions reduces and closes the account's own positions at the market's
price, from their rows, and says the take profit and stop loss that wait on
each. Every position it shows, anybody's, is valued at its market's live
price between two reports of its account: on Hyperliquid the middle of the
best bid and offer, each time either changes (several times a second for a
busy market), or the exchange's own mark price if the settings ask for it
(about once a second, which is Hyperliquid's pace). The Trade Panel shows
the same live price beside its market. Prices have a source of their own
per exchange, apart from accounts and trading. Both
widgets are behind an exchange-neutral trading layer, as the portfolio layer
is for reading. Trading is enabled per exchange with one signature in the
wallet, which approves an agent key the app keeps on this device, so
trading stays enabled between runs: connecting the account again is enough
while the exchange still lists the agent, and disabling trading forgets it.
The agent can place and cancel orders and cannot withdraw; its key is saved
in the app's storage in the clear, as Hyperliquid's own site keeps its
agent. An account that trades is followed live over the exchange's
WebSocket: its open orders, margin and fills arrive as the market changes
them, into the same shared requests every widget watching the account
reads, with polling as the fallback. No exchange SDK is linked: each
exchange's signing and wire format is written over alloy and checked
against that exchange's own SDK's test vectors. On desktop the platform's
normal title bar keeps minimize,
maximize/restore, and close; on web the browser owns the window chrome.

Before changing code, read `docs/nsh-development.md`, the development guide
(architecture, data layer, widgets, conventions). Before building or
restyling a widget's view, a panel or a menu, read `docs/design.md`, the
visual design guide (flush blocks, dividers, menus, sizes, states, and the
code to copy). `docs/` is local only and not tracked by git.

## NSH project

`nsh.json` is the app configuration. It declares the Rust crate and `App` root;
NSH uses that same root for its desktop runners and its web runtime.

```json
{
  "name": "Lattice",
  "id": "com.latticetrading.lattice",
  "version": "0.1.0",
  "build": 1,
  "rust": ".",
  "root": "App",
  "appearance": "dark",
  "window": {
    "restoreBounds": true
  }
}
```

`window.restoreBounds` has the desktop window open where it was last left
(size, position and maximized state, kept in the app's storage).

The web package is a static site generated by NSH:

```sh
nsh package . --target web
nsh package . --target web --run
```

The native desktop package uses the target's NSH runner. On Windows:

```sh
nsh run . --target windows
nsh package . --target windows --run
```

`cargo run` remains a direct desktop smoke launcher and mounts the same `App`
root. NSH's Rust app workflow requires its platform toolchain; web packaging
also requires the WebAssembly toolchain and `clang`/`llvm-ar` as described in
the NSH documentation, plus the `wasm-bindgen` CLI at the version in
`Cargo.lock` (the browser WebSocket goes through `web-sys`).

## Checks

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The repository toolchain is Rust 1.99.1 and includes the
`wasm32-unknown-unknown` target. Do not launch GUI smoke tests from automation;
visual and native-caption acceptance remains a user-run check.

## Source map

- `nsh.json`: NSH app identity, Rust root, and appearance.
- `rustfmt.toml`: the 2024 edition and style, so a file formatted by itself matches `cargo fmt`.
- `themes/`: one JSON file per theme.
- `src/lib.rs`: the registered NSH UI set and shared `App` root.
- `src/main.rs`, `src/app.rs`: optional direct Cargo desktop launcher.
- `src/shell.rs`, `src/shell/`: the shell, Everything Bar (with its symbol search, link button, wallet and notifications' bell), Settings panel, clock, and the notification popups.
- `src/notify.rs`, `src/notify/`: the notifications' center, which anything posts to, and how the user has them pop up.
- `src/ui.rs`, `src/ui/`: shared controls (text field, button, tooltip hint, settings rows, floating panels, the link button, list scrolling).
- `src/layout.rs`, `src/layout/grid.rs`: workspace pages, tabs, panes, and responsive geometry.
- `src/storage.rs`: versioned NSH storage-service persistence.
- `src/theme.rs`, `src/theme/roles.rs`: the theme engine and its color roles.
- `src/assets.rs`, `assets/`: compile-time fonts and app assets (icons, exchange logos).
- `src/data.rs`, `src/data/`: the data layer (`net/`: shared HTTP, WebSocket, polling, live-feed and rate-limit plumbing; `ferris/`: the Ferris integration, with per-exchange rules in `ferris/venues.rs` and the DefiLlama data it proxies in `ferris/defillama.rs`; `hyperliquid/`: Hyperliquid's own API, asked by the app itself within its rate limit; `lighter/`: Lighter's, asked the same way; `txflow/`: TxFlow's, the same again; `portfolio/`: what an account holds and has made, in one shape whatever its exchange, the trait an exchange's integration implements to report accounts, and several accounts' reports combined into one; `trading.rs`, `trading/`: orders for the connected wallet's account in one shape whatever the exchange, and the trait an exchange's integration implements to be traded on (Hyperliquid's is `hyperliquid/trading.rs`, with `exchange.rs`, `signing.rs`, `agent.rs` and `pack.rs`, and `stream.rs` with `live.rs` for its WebSocket); `session.rs`: per-widget data sessions; `markets.rs`: the app's one directory of every market, which the search and the market picker read; `wallet.rs`, `wallet/`: the user's browser wallet through alloy, `web.rs` for the page's own wallets and `bridge.rs` with `bridge.html` for the desktop's page in the user's browser).
- `src/widgets.rs`, `src/widgets/`: the widget registry, the widget picker, shared widget parts (`common/`: market picker, exchange logos, number field, double clicks, in `common/books/` what the widgets that draw order books share, in `common/plot/` the GPU plot the charts share, in `common/accounts/` what the widgets that show an account share, and in `common/access.rs` the rows that connect a wallet and enable an exchange for it), and one folder per widget (`account_performance/`, `account_summary/`, `chart/`, `defi_protocols/`, `depth_chart/`, `funding_rates/`, `market_screener/`, `order_book/`, `positions/`, `premium_arb_chart/`, `price_delta_monitor/`, `protocol_tvl/`, `trade_history/`, `trade_panel/`, `trade_stream/`).
- `src/link.rs`: color linking (the link colors, and the Everything Bar's).
- `src/exchanges.rs`, `src/format.rs`: exchange names and logos; number formatting.
- `src/platform/`: future native integrations (empty).

## NSH documentation used for this migration

- Native Shell project structure and `nsh.json` configuration
- Building Rust apps and shared roots
- Desktop windows and title bars
- WebAssembly runtime and web packaging
- Styles, fonts, assets, storage, scrolling, and touch/press input

The canonical docs are at <https://nsh.dev/docs> and the NSH repository at
<https://github.com/longbridge/nsh>.
