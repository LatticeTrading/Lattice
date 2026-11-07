# Initial native shell

## Scope and reference

This slice targets Windows. It is application chrome plus the first native
workspace migration checkpoint, not a trading demo. It stores only this Rust
application's versioned workspace file; it neither reads nor modifies the
previous application's browser/localStorage data.

Reviewed `../lattice-pro` at commit
`ac999e00567b6a1558715d30f4b38730488e2e9f` (the reference checkout also had
unrelated uncommitted chart changes; none were modified).

Sources of truth:

- `docs/everything-bar.md`: placement, order, shared-edge controls, clock/status.
- `src/components/layout/EverythingBar.tsx`: legacy rendered controls/text.
- `src/renderer/styles/components/layout/EverythingBar.css`.
- `src/renderer/styles/theme.css` and `globals.css`: dark palette and IBM Plex Mono.
- `src/components/layout/ConnectionStatusIndicator.tsx`: legacy connection icon.
- The Rust implementation owns the current page controls and persistence.

Where the prose is stale, the mounted source wins: it does not render the wallet
selector, and the current canonical font is IBM Plex Mono, not Geist Mono.

## Visible contract

- Empty dark workspace (`#0a0a0a`) on first run, with no instructional/
  marketing/empty-state copy. Later runs restore the saved panes and tabs.
- Bar inset 8 logical pixels, height 40, square edges, 1px shared dividers,
  surface `#1a1a1a`, border `#2a2a2a`.
- Order: **LATTICE**, **Layout**, numbered layout pages, **+**, search,
  **+ New Widget**, clock, connection icon, settings, minimize,
  maximize/restore, close.
- The first numbered page is a real persisted workspace page. Clicking a page
  switches the visible workspace; **+** creates and selects a new blank page.
  There is no profile/preset import or clear-layout manager yet.
- Layout `+` matches the numbered page button's dimensions. It and the
  content-sized New Widget button have explicit right edges, so neither blends
  into the wide-window gaps. Adjacent segments share one border, not two.
- Search retains `Search symbols...`, without an invented selected market.
- Clock uses `--:--:--`, not a frozen sample time. No timer is started.
- Connection icon is neutral gray. No online/offline claims or fabricated counts.
- No unused color-link groups, updater chip, wallet control, menus or dropdowns.
- The Layout manager placeholder and Kit input remain disabled. Page controls
  and New Widget are active. Native window controls remain usable.
- Initial client area: 1280 × 800; minimum 640 × 360. At <=768px, icon segments
  become 36px and page segments 28px; search yields space first. This slice is
  desktop-only and does not reproduce the old <=480px mobile hiding rules.
- The wordmark and clock are native Windows drag regions. Native window-control
  hit regions preserve OS caption behavior; Kit callbacks support keyboard
  activation. Other platforms' custom chrome still needs explicit validation.

## Implementation boundaries

One package. `app.rs` owns lifecycle, `Shell` owns its retained input and
workspace subscription, `EverythingBar` composes Kit controls, `layout.rs` owns
page/grid interaction, and `storage.rs` owns the versioned local workspace file.
No event bus, service stubs, fake repositories, new component framework, or
speculative widget abstractions.

GPUI Kit's root is installed through `gpui_kit::open_window`. Font/startup
errors reach `main` with context and a nonzero exit. The last native window
closing quits the application. Icons and font bytes are embedded at compile
time; the only runtime file access is the versioned local workspace JSON.

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
- disabled layout/settings/connection semantics, disabled search state and no focus on click;
- empty search value, placeholder, clock accessibility label;
- active page creation and widget creation without opening extra windows;
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

1. `cargo run --locked`: one window named Lattice. On first run the workspace
   is empty; on later runs the last saved pages and panes should return. No
   console errors, unexpected dialogs, dropdowns or remote requests.
2. Resize to 1280px, 768px and the 640px minimum. All native controls must remain
   visible; search may truncate. The input icon/text must be vertically centered.
3. Try Layout, page/add, New Widget, connection, settings and search. Layout,
   connection, settings and search remain unavailable; page/add and New Widget
   create/switch persisted workspace content. Tab should skip unavailable
   product controls.
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
