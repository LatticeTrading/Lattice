# Lattice

Native Rust trading terminal rewrite using **GPUI Kit 0.9.0 with GPUI Fast 0.1.13**.
One Cargo package, with one root `Cargo.toml` and a reproducible dependency lockfile.

The current slice is a startable Windows desktop shell with a working multi-page
workspace. Panes, tabs, responsive grid geometry, the active page, and page
creation are persisted locally so the terminal reopens where it was left. The
trading widgets and backend connections are still migration scaffolds; profile
keys, preset imports, and the layout-clear manager are intentionally not in this
slice. See [the shell contract](docs/initial-shell.md).

## Run (Windows)

Prerequisites: [rustup](https://rustup.rs/) and Visual Studio 2022's **Desktop
development with C++** workload, including the MSVC toolset and Windows SDK.
A graphics driver supporting GPUI's Direct3D renderer is required.
`rust-toolchain.toml` selects Rust 1.99.1 for this repository, without changing
your global default. Rustup installs that toolchain on first use.

```sh
cargo run --locked
```

The first build downloads and compiles the native UI stack and can take several
minutes. Fonts and icons are embedded; the running application needs no network
or assets directory. The binary is `target/debug/lattice.exe`.

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
```

`cargo build --locked --release` creates an optimized binary. Windows release
shader compilation also needs the SDK's `fxc.exe` (normally discovered by GPUI;
`GPUI_FXC_PATH` can specify it). Release packaging/signing and other operating
systems are not validated by this first Windows slice.

## Source map

- `src/app.rs`: startup, error propagation, native window options and shutdown.
- `src/theme.rs`: initial dark palette and embedded font registration.
- `src/assets.rs`: app-owned icons composed with Kit's asset bundle.
- `src/shell.rs`, `src/shell/everything_bar.rs`: shell composition and bar presentation.
- `src/shell/tests.rs`: headless layout, control and initial-window contract tests.
- `assets/`: embedded fonts/icons and their license/provenance notes.
- `src/widgets/`: named product surfaces to assess during migration.
- `src/data/`: backend communication, market identity, acquisition, subscriptions, and shared data ownership.
- `src/layout.rs`, `src/layout/grid.rs`: workspace pages, tabs, widget placement, and responsive grid geometry.
- `src/storage.rs`: versioned local workspace persistence with atomic replacement.
- `src/platform/`: native integration such as dialogs, external links, and updates.

These folders are navigation hints, not mandatory architecture boundaries.
Choose internal files, Rust modules, and shared abstractions after inspecting
actual behavior and the needs of the native implementation. No view/state/controller
split or separate rendering engine is prescribed. Use gpui-kit where appropriate
rather than prebuilding another component library.

## Widget migration references

Reference: `../lattice-pro/src/components/widgetRegistry.ts`.
These are existing product names, not a guarantee that every legacy feature
will ship or that every folder must become an independent implementation.

| Folder under `src/widgets/` | Previous widget(s) |
| --- | --- |
| `chart/` | Chart |
| `order_book/` | Order Book, Vertical Order Book, WebGL Order Book |
| `depth_chart/` | Depth Chart, Vertical Depth Chart |
| `market_screener/` | Market Screener |
| `spot_screener/` | Spot Screener |
| `options_screener/` | Options Screener |
| `funding_rates/` | Funding Rates |
| `trade_stream/` | Trade Stream |
| `liquidation_feed/` | Liquidation Feed |
| `price_delta_monitor/` | Price Delta Monitor |
| `premium_arb_chart/` | Premium/Arb Chart |
| `market_data/` | Market Data |
| `trade_panel/` | Trade Panel |
| `positions/` | Positions |
| `wallet/` | Wallet |
| `news/` | News Feed |
| `console/` | Console |

Order-book and depth-chart variants are grouped for discovery, not a decision
to merge their behavior. Preserve relevant persisted widget IDs when importing
old layouts. WebGL is an old implementation detail, not a requirement for Rust.
Screeners may share code; their folders do not require duplicate implementations.

Assess functionality before porting: the old architecture documents no functional
authenticated account backend or order placement/cancellation, and identifies the
wallet as active legacy UI. Presence in the registry does not prove live data or
execution support. These areas need explicit scope decisions, not blind copying.

## Guidance for implementation

Inspect the old feature and its consumers, identify what should be preserved,
then design for Rust and GPUI. Add files only when there is a concrete responsibility
to own. Keep related code together; split at meaningful boundaries before files
become unwieldy. Renaming, combining, or removing scaffold folders is expected.
Prefer named module roots (`widgets.rs`, `widgets/chart.rs`) over `mod.rs` when
modules are introduced. Do not add internal Cargo packages without an explicit decision.

`.gitkeep` files only preserve empty directories in Git; remove each when real
files are added. Remaining feature directories are still planning scaffolds.

Depend on GPUI through `gpui-kit`, with its `gpui-fast` feature. Do not add a
second direct GPUI dependency or patch the registry sources. Kit enables Fast
across its layers; Cargo also builds upstream GPUI dependencies because features
are additive. Upgrade Kit/backend/toolchain deliberately, with the tests and
native smoke checklist, and retain `Cargo.lock` for this application.
