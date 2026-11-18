use nsh_core::{Checked, element::Root, prelude::*};
use nsh_testing::{Headless, WAIT};
use serde_json::json;

use super::{
    session::{self, EMPTY, LOADING, Readout, UNAVAILABLE},
    settings::{AccountPerformanceSettings, Choice, Metric, Style},
};
use crate::{
    data::portfolio::{Period, Point, Scope, Unavailable, Window},
    widgets::{
        WidgetKind,
        common::{
            accounts::{
                AccountSettings, NONE, Standing, Tone,
                testing::{IDLE, USER, account, asked, count, start, workspace_of},
            },
            plot::{Mode, Scene, Viewport},
        },
    },
};

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    // Defaults save nothing: no account, a month of the whole account's
    // value, with the grid.
    let defaults = AccountPerformanceSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert_eq!(
        (defaults.period(), defaults.scope(), defaults.metric()),
        (Period::Month, Scope::Total, Metric::Value)
    );
    assert!(defaults.grid() && defaults.account().is_none());
    let written = json!({
        "period": "month", "scope": "total", "metric": "value", "style": "line", "showGrid": true,
    });
    assert_eq!(AccountPerformanceSettings::from(written), defaults);

    // Each choice under its id, beside the account.
    let mut saved = account(USER);
    saved["period"] = json!("allTime");
    saved["scope"] = json!("perps");
    saved["metric"] = json!("pnl");
    saved["style"] = json!("area");
    saved["showGrid"] = json!(false);
    let settings = AccountPerformanceSettings::from(saved.clone());
    assert_eq!(
        (settings.period(), settings.scope(), settings.metric()),
        (Period::AllTime, Scope::Perps, Metric::Pnl)
    );
    assert!(!settings.grid());
    assert_eq!(settings.style(), Style::Area);
    assert_eq!(settings.account().unwrap().address, USER);
    assert_eq!(serde_json::to_value(&settings).unwrap(), saved);
    // Back at its default a choice is not saved.
    assert_eq!(
        serde_json::to_value(
            settings
                .with_period(Period::Month)
                .with_scope(Scope::Total)
                .with_metric(Metric::Value)
                .with_style(Style::Line)
                .with_grid(true)
        )
        .unwrap(),
        account(USER)
    );

    // A choice it does not know, or of the wrong type, is the default.
    let damaged = AccountPerformanceSettings::from(json!({
        "period": "year", "scope": 7, "metric": null, "style": "bars", "showGrid": "no",
    }));
    assert_eq!(damaged, defaults);
    // Every choice reads back from what it saves.
    assert!(
        Period::ALL
            .iter()
            .all(|period| Period::from_id(period.id()) == Some(*period))
    );
    assert!(
        Scope::ALL
            .iter()
            .all(|scope| Scope::from_id(scope.id()) == Some(*scope))
    );
}

fn window(value: &[(f64, f64)], pnl: &[(f64, f64)], volume: Option<f64>) -> Window {
    let points = |samples: &[(f64, f64)]| {
        samples
            .iter()
            .map(|(time_ms, value)| Point {
                time_ms: *time_ms,
                value: *value,
            })
            .collect()
    };
    Window {
        period: Period::Day,
        scope: Scope::Total,
        value: points(value),
        pnl: points(pnl),
        volume,
    }
}

#[test]
fn a_window_plots_as_a_bar_a_sample_and_reads_what_it_made() {
    let window = window(
        &[
            (1_000.0, 100.0),
            (61_000.0, 104.5),
            (61_400.0, 105.0),
            (121_999.0, 99.0),
        ],
        &[(1_000.0, 2.0), (61_000.0, 6.5), (121_000.0, 1.0)],
        Some(2_500.75),
    );
    // The value: whole seconds, the later of two samples in one second.
    let bars = session::bars_of(&window, Metric::Value);
    let plotted: Vec<(f64, f64)> = bars.iter().map(|bar| (bar.time, bar.close)).collect();
    assert_eq!(plotted, [(1.0, 100.0), (61.0, 105.0), (121.0, 99.0)]);
    assert!(
        bars.iter()
            .all(|bar| bar.open == bar.close && bar.high == bar.close && bar.low == bar.close)
    );
    // The profit and loss is another series of the same window.
    let made = session::bars_of(&window, Metric::Pnl);
    assert_eq!(made.len(), 3);
    assert_eq!(made[1].close, 6.5);

    // What the window made is counted from its first sample.
    let readout = Readout::of(Some(&window));
    assert_eq!(readout.pnl, "-$1.00");
    assert_eq!(readout.tone, Some(Tone::Loss));
    assert_eq!(readout.volume, "$2,500.75");

    // Nothing sampled, or no window: dashes, and nothing to color.
    let quiet = Readout::of(Some(&self::window(&[], &[], None)));
    assert_eq!((quiet.pnl.as_str(), quiet.volume.as_str()), (NONE, NONE));
    assert_eq!(quiet.tone, None);
    assert_eq!(Readout::of(None), quiet);
    assert!(session::bars_of(&self::window(&[], &[], None), Metric::Value).is_empty());
}

#[test]
fn the_line_is_green_above_its_baseline_and_red_below_it() {
    let window = window(
        &[(1_000.0, 100.0), (61_000.0, 90.0)],
        &[(1_000.0, 2.0), (61_000.0, -3.0)],
        None,
    );
    // Profit and loss about nothing; value about where the window began.
    assert_eq!(session::baseline_of(Some(&window), Metric::Pnl), 0.0);
    assert_eq!(session::baseline_of(Some(&window), Metric::Value), 100.0);
    assert_eq!(session::baseline_of(None, Metric::Value), 0.0);
    assert_eq!(session::mode_of(Style::Line), Mode::Signed);
    assert_eq!(session::mode_of(Style::Area), Mode::SignedArea);

    // The newest value's mark takes the color of its side.
    let scene = |metric: Metric, style: Style| {
        let bars = session::bars_of(&window, metric);
        Scene::build(
            &bars,
            &Viewport::showing(2.0),
            session::mode_of(style),
            200.0,
            100.0,
        )
        .with_baseline(session::baseline_of(Some(&window), metric))
    };
    assert!(!scene(Metric::Pnl, Style::Line).last.unwrap().rising);
    assert!(!scene(Metric::Value, Style::Area).last.unwrap().rising);
    let flat = Scene::build(
        &session::bars_of(&window, Metric::Value),
        &Viewport::showing(2.0),
        Mode::Signed,
        200.0,
        100.0,
    );
    assert!(flat.last.unwrap().rising, "90 is above nothing");
}

#[test]
fn the_plot_says_why_it_has_no_point_and_nothing_once_it_has_one() {
    assert_eq!(session::said(&Standing::Loading, false), LOADING);
    assert_eq!(session::said(&Standing::Ready, false), EMPTY);
    assert_eq!(
        session::said(&Standing::Stale("offline".to_owned()), false),
        EMPTY
    );
    assert_eq!(
        session::said(&Standing::Failed("offline".to_owned()), false),
        UNAVAILABLE
    );
    assert_eq!(
        session::said(&Standing::Unavailable(Unavailable::NotReported), false),
        UNAVAILABLE
    );
    assert_eq!(session::said(&Standing::Unset, false), "");
    assert_eq!(session::said(&Standing::Ready, true), "");
    assert_eq!(session::said(&Standing::Loading, true), "");
}

/// A workspace of one Account Performance watching the stubbed account.
#[component]
fn Watching() -> impl IntoElement {
    workspace_of(WidgetKind::AccountPerformance, Some(account(USER)))
}

/// One watching an account that never traded.
#[component]
fn Idle() -> impl IntoElement {
    workspace_of(WidgetKind::AccountPerformance, Some(account(IDLE)))
}

/// Whether the bar's figure `id` reads `read`.
fn reads(app: &Headless, id: &str, read: &str) -> bool {
    app.screen()
        .matches(&format!("id:account-performance-{id} >> text:{read}"))
        .unwrap()
}

#[test]
fn the_plot_shows_the_window_its_settings_pick_of_one_report() {
    let (mut app, network) = start(Root::of::<Watching>());

    // A month of the whole account's value by default: the newest value's
    // tag, and what the month made and traded under the header.
    app.wait_for("id:account-performance-last-price").unwrap();
    app.wait_for("12450.0").unwrap();
    assert!(reads(&app, "pnl", "+$2,450.00"));
    assert!(reads(&app, "volume", "$250,000.50"));
    assert!(!app.shows(LOADING) && !app.shows(EMPTY));
    // Its samples are hours apart: the axis names their day too.
    assert!(app.shows("Mar 13 17:23"));
    // The header names the account and the period; there is no strip or
    // bar besides.
    assert!(
        app.screen()
            .matches(
                "id:account-performance-header >> label:Change account: Hyperliquid 0x0123…4567"
            )
            .unwrap()
    );
    assert!(reads(&app, "period", "30D"));
    assert!(!app.screen().matches("id:account-performance-bar").unwrap());

    // The period's badge opens the menu of them, the one plotted checked;
    // another is another window of the same report.
    app.press("id:account-performance-period").unwrap();
    app.wait_for("id:account-performance-period-menu").unwrap();
    let checked = |app: &Headless, period: &str| {
        app.screen()
            .accessibility_state(&format!("id:account-performance-period-{period}"))
            .unwrap()
            .checked
            == Some(Checked::True)
    };
    assert!(checked(&app, "month") && !checked(&app, "day"));
    app.press("id:account-performance-period-day").unwrap();
    app.wait_for_gone("id:account-performance-period-menu", WAIT)
        .unwrap();
    app.wait_for("13058.0").unwrap();
    assert!(reads(&app, "pnl", "+$58.00"));
    assert!(reads(&app, "period", "24H"));

    // So is the account's perpetuals alone, and what it made instead of
    // what it was worth, picked in the settings panel, which holds the
    // account too.
    app.press("Widget settings").unwrap();
    app.wait_for("id:account-performance-settings").unwrap();
    assert_eq!(
        app.screen()
            .input_text("id:account-performance-settings >> id:account-performance-address")
            .unwrap(),
        Some(USER)
    );
    app.press("id:account-performance-settings >> label:Of")
        .unwrap();
    app.press("role:option[Perpetuals]").unwrap();
    app.wait_for("8971.00").unwrap();
    assert!(reads(&app, "pnl", "-$29.00"));
    app.press("id:account-performance-settings >> label:Plot")
        .unwrap();
    app.press("role:option[PnL]").unwrap();
    app.wait_for("-29.000").unwrap();
    app.press("label:Close Account Performance settings")
        .unwrap();
    app.wait_for_gone("id:account-performance-settings", WAIT)
        .unwrap();

    // All time runs over years: the axis names each day with its year.
    app.press("id:account-performance-period").unwrap();
    app.press("id:account-performance-period-allTime").unwrap();
    app.wait_for("Mar 13 2025").unwrap();
    assert!(!app.shows("Mar 13 17:23"));
    assert!(reads(&app, "period", "All"));

    // One request carried every window, and nothing else was asked for.
    let requests = asked(&network);
    assert_eq!(count(&requests, "portfolio", USER), 1);
    assert_eq!(requests.len(), 1);
}

#[test]
fn the_account_badge_opens_the_rows_that_change_the_account() {
    let (mut app, network) = start(Root::of::<Watching>());
    app.wait_for("12450.0").unwrap();
    app.press("id:account-performance-account").unwrap();
    app.wait_for("id:account-performance-account-panel")
        .unwrap();
    const FIELD: &str = "id:account-performance-account-panel >> id:account-performance-address";
    assert_eq!(app.screen().input_text(FIELD).unwrap(), Some(USER));

    // Another account saved: it is watched in its place, and the panel
    // has done its work.
    app.clear(FIELD).unwrap();
    app.type_text(FIELD, &format!("{IDLE}\n")).unwrap();
    app.wait_for_gone("id:account-performance-account-panel", WAIT)
        .unwrap();
    app.wait_for(EMPTY).unwrap();
    assert!(
        app.screen()
            .matches("label:Change account: Hyperliquid 0x0000…00aa")
            .unwrap()
    );
    assert_eq!(count(&asked(&network), "portfolio", IDLE), 1);

    // Its close button closes it with nothing changed.
    app.press("id:account-performance-account").unwrap();
    app.wait_for("id:account-performance-account-panel")
        .unwrap();
    app.press("label:Close Account").unwrap();
    app.wait_for_gone("id:account-performance-account-panel", WAIT)
        .unwrap();
    app.wait_for(EMPTY).unwrap();
}

#[test]
fn an_account_with_no_history_says_so_in_the_plot() {
    let (mut app, _network) = start(Root::of::<Idle>());
    app.wait_for(EMPTY).unwrap();
    assert!(reads(&app, "pnl", NONE));
    assert!(reads(&app, "volume", "$0.00"));
    assert!(
        !app.screen()
            .matches("id:account-performance-last-price")
            .unwrap()
    );
}
