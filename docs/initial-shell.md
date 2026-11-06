# Initial native shell

## Scope and reference

This slice targets Windows. It is application chrome, not a trading demo.
It neither reads nor modifies the previous application's data.

Reviewed `../lattice-pro` at commit
`ac999e00567b6a1558715d30f4b38730488e2e9f` (the reference checkout also had
unrelated uncommitted chart changes; none were modified).

Sources of truth:

- `docs/everything-bar.md`: placement, order, shared-edge controls, clock/status.
- `src/components/layout/EverythingBar.tsx`: actually rendered controls/text.
- `src/renderer/styles/components/layout/EverythingBar.css`.
- `src/renderer/styles/components/layout/LayoutMenu.css`.
- `src/renderer/styles/components/ui/LayoutPresetButtons.css`.
- `src/renderer/styles/theme.css` and `globals.css`: dark palette and IBM Plex Mono.
- `src/components/layout/ConnectionStatusIndicator.tsx`: connection icon.

Where the prose is stale, the mounted source wins: it does not render the wallet
selector, and the current canonical font is IBM Plex Mono, not Geist Mono.

## Visible contract

- Empty dark workspace (`#0a0a0a`), no instructional/marketing/empty-state copy.
- Bar inset 8 logical pixels, height 40, square edges, 1px shared dividers,
  surface `#1a1a1a`, border `#2a2a2a`.
- Order: **LATTICE**, **Layout**, **1**, **+**, search, **+ New Widget**,
  clock, connection icon, settings, minimize, maximize/restore, close.
- `1` is the sole blank initial page marker, not an implemented/saved preset.
- Layout `+` matches the numbered page button's dimensions. It and the
  content-sized New Widget button have explicit right edges, so neither blends
  into the wide-window gaps. Adjacent segments share one border, not two.
- Search retains `Search symbols...`, without an invented selected market.
- Clock uses `--:--:--`, not a frozen sample time. No timer is started.
- Connection icon is neutral gray. No online/offline claims or fabricated counts.
- No unused color-link groups, updater chip, wallet control, menus or dropdowns.
- Product buttons and the Kit input are disabled; no no-op action handlers
  pretend to implement features. Native window controls remain usable.
- Initial client area: 1280 × 800; minimum 640 × 360. At <=768px, icon segments
  become 36px and page segments 28px; search yields space first. This slice is
  desktop-only and does not reproduce the old <=480px mobile hiding rules.
- The wordmark and clock are native Windows drag regions. Native window-control
  hit regions preserve OS caption behavior; Kit callbacks support keyboard
  activation. Other platforms' custom chrome still needs explicit validation.

## Implementation boundaries

One package. `app.rs` owns lifecycle, `Shell` owns its retained input entity,
`EverythingBar` composes Kit controls, and `theme.rs` configures Kit plus the
small set of legacy shell tokens. No event bus, service stubs, fake repositories,
new component framework, or speculative widget/layout abstractions.

GPUI Kit's root is installed through `gpui_kit::open_window`. Font/startup
errors reach `main` with context and a nonzero exit. The last native window
closing quits the application. Icons and font bytes are embedded at compile
time; there are no runtime filesystem or network asset dependencies.

The search is a real disabled Kit Input, preserving its state/IME foundation for
later work. Kit 0.9's test projection reports no disabled accessibility flag for
the input's semantic owner, even though input behavior is disabled. Tests check
the actual input presentation and lack of pointer focus rather than claiming
that missing flag is correct. Screen-reader acceptance remains outstanding.

## Automated checks

Run the commands in the root README. The Windows CI job performs the same gates.
Tests cover:

- window title, initial dimensions, minimum size and resizability;
- bar bounds, full-height search and control geometry at 1920/1600/1280/768/640px;
- fixed page/add dimensions, content-sized New Widget, wide-window gaps and
  single shared-edge placement at narrower widths (not rendered border pixels);
- disabled product-button semantics, disabled search state and no focus on click;
- empty search value, placeholder, clock accessibility label;
- inert placeholder activation (no page creation or new windows);
- native control roles and names;
- availability of every shell icon from embedded assets.

Headless tests do **not** establish GPU pixels, native caption hit testing,
monitor scaling, screen-reader behavior, or release packaging.

## Windows smoke checklist (user-run only)

The workstation is in active use. Agents must not launch the GUI, capture
screenshots, move the pointer, or manipulate desktop windows unless the user
explicitly requests it. Use non-interactive build/lint/headless checks instead;
the user owns visual acceptance and reports design changes. The historical
native checks below were performed before this restriction was established.

1. `cargo run --locked`: one window named Lattice, empty workspace, bar only.
   No console errors, unexpected dialogs, dropdowns or remote requests.
2. Resize to 1280px, 768px and the 640px minimum. All native controls must remain
   visible; search may truncate. The input icon/text must be vertically centered.
3. Try Layout, page/add, New Widget, connection, settings and search: no changes
   or new content. Tab should skip unavailable product controls.
4. Drag the wordmark/clock, double-click a drag region, use minimize and restore,
   maximize and restore, and test Windows snap. Buttons must not drag the window.
5. Check keyboard navigation/activation of native controls, Alt+F4, and the X.
   Closing the final window must end the process.
6. Repeat a visual sample at 100% and 150% display scaling; check the bar edges,
   text, icons and focus indicator. Check Narrator separately.
7. Run the built executable from a different working directory to confirm fonts
   and icons are embedded. Ship font/icon license notices with any distribution.

Initial local validation (2026-11-05): format, check, debug build, three tests
and warning-free Clippy passed on `x86_64-pc-windows-msvc`. Launched the binary
from a different working directory and captured native rendering at
1280/768/640px. Native Windows hit tests returned the correct caption,
client-only Layout, minimize, maximize and close regions at all three widths.
OS minimize/restore/maximize/restore succeeded, and native close terminated the
process with no stderr output. These are programmatic smoke checks, not a full
manual interaction pass. Release builds, CI execution, other operating systems,
DPI variants, manual dragging/snap and accessibility acceptance remain separate
gates, not implied by a passing unit suite.
