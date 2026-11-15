# Embedded assets

## IBM Plex Mono

Unmodified Regular (400), Medium (500), SemiBold (600), and Bold (700) TTFs from
https://github.com/google/fonts/tree/main/ofl/ibmplexmono, matching the font in
lattice-pro's canonical `theme.css` / `globals.css`. Retrieved 2026-11-05.

Copyright IBM Corp.; SIL Open Font License 1.1. The complete license is in
`fonts/ibm-plex-mono/OFL.txt`. Include that file with binary distributions.
No font installation or runtime download is required.

SHA-256:

| File | SHA-256 |
| --- | --- |
| IBMPlexMono-Regular.ttf | 6a3412f058c7d8dfd9170c41e85ade48e5156ecb89356110ca57a0a27734af46 |
| IBMPlexMono-Medium.ttf | a9b4c49bb299e05b5f6c481e7fb5e78943d2793249a0c8874ab574a2d1ea6755 |
| IBMPlexMono-SemiBold.ttf | d3c38e55c78f5b0f28009fddba4834ec503278936a5986032424c9bd2d23aa46 |
| IBMPlexMono-Bold.ttf | ac27abd6450a64dd94467580a02fe6235156d5b92f2926ebbc8e7489df64e0be |

## Icons

`icons/connection-status.svg` preserves the path/circle geometry of
`lattice-pro/src/components/layout/ConnectionStatusIndicator.tsx` at
`ac999e00567b6a1558715d30f4b38730488e2e9f`.

`icons/search.svg` is the magnifier of the old search field
(`lattice-pro/src/components/layout/EverythingBar.tsx`, its `.search-icon`
path), drawn in `currentColor`.

`icons/plus.svg` and `icons/remove.svg` are the "Add pair" plus and the row
× of the old Funding Rates table
(`lattice-pro/src/components/trading/FundingRates.tsx`, its inline SVG
paths), drawn in `currentColor`.

`icons/cursor.svg`, `icons/trend-line.svg`, `icons/fib-retracement.svg`,
`icons/measure.svg`, `icons/trash.svg`, `icons/chevron-left.svg` and
`icons/chevron-right.svg` are the Chart's drawing toolbar's (its tools,
removing drawings, and closing and opening it): Lattice's own, paths only,
drawn in `currentColor`; `src/widgets/chart/drawings.rs` and
`src/widgets/chart/drawings/toolbar.rs` inline them.

`icons/window-minimize.svg`, `icons/window-maximize.svg` and
`icons/window-close.svg` are the old Everything Bar's window buttons
(`lattice-pro/src/components/layout/EverythingBar.tsx`, its
`.window-control-btn` paths, the maximize square as a path);
`icons/window-restore.svg` is Lattice's own, in the same 12pt box and
stroke. All are drawn in `currentColor`.

They are the only icon files; other controls draw text glyphs (`⚙`, `+`).
What uses them inlines them at compile time (`include_str!` in
`src/shell/everything_bar.rs`, `src/shell/everything_bar/search.rs` and
`src/widgets/funding_rates/view.rs`); `src/assets.rs` registers the fonts. No loose
files are read at runtime. `LICENSE-LUCIDE` is left from the GPUI build,
whose Lucide icons (`gpui-kit-assets`) Lattice no longer bundles.

## Exchange logos

`exchanges/*.svg` are generated from `EXCHANGE_LOGO_ASSETS` in
`lattice-pro/src/components/ui/exchangeLogoAssets.ts` at
`ac999e00567b6a1558715d30f4b38730488e2e9f`: shapes, viewBoxes and colors
unchanged, one file per exchange. One deliberate change: the Lighter mark's
`#06060C` fill is `currentColor`, so it follows the text color instead of
vanishing on dark themes. `src/exchanges.rs` inlines them at compile time.
