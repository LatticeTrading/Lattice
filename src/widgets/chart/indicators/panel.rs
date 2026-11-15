//! An indicator's settings panel, below its row's gear in the legend and
//! laid out as the chart's own ([`crate::widgets::chart::panel`]): a row a
//! setting, its name on the left and its control on the right, every
//! change saved in the chart's settings at once.
//!
//! - A row an input of the indicator's kind ([`Input`]): the compact
//!   number field for one of whole numbers (a length), a compact select of
//!   the values it takes for another (a multiplier).
//! - Color: a compact select of the colors an indicator can be drawn in
//!   ([`COLORS`]).

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{COLORS, Indicator, Input, change};
use crate::{
    link::LinkColor,
    ui::{self, Side},
    widgets::{TabContext, chart::ChartSettings, common::number_field},
};

/// The panel's width.
const PANEL_WIDTH: Points = px(240.0);

/// The settings panel of the indicator `at` of the chart in `tab`, below
/// `anchor` (its row's gear); none once the chart has no such indicator.
/// Its close button and Escape call `on_close`.
pub(super) fn panel(
    tab: &TabContext<ChartSettings>,
    at: usize,
    anchor: &ElementRef,
    on_close: impl Fn() + Clone + 'static,
) -> Option<Overlay> {
    let saved = tab.settings;
    let kind =
        saved.with_untracked(|settings| settings.indicators().get(at).map(Indicator::kind))?;
    let inputs = kind.inputs().iter().enumerate();
    let body = view()
        .test_id("chart-indicator-settings")
        .flex_col()
        .py(px(4.0))
        .children(inputs.map(|(number, input)| input_row(tab.clone(), at, number, *input)))
        .child(color_row(tab.clone(), at));
    Some(ui::settings_panel(
        anchor,
        Side::Left,
        PANEL_WIDTH,
        &format!("{} settings", kind.label()),
        on_close,
        body,
    ))
}

/// The row of `input`, the `number`th of the indicator `at`'s kind.
fn input_row(tab: TabContext<ChartSettings>, at: usize, number: usize, input: Input) -> View {
    let saved = tab.settings;
    let value = move || {
        saved.with(|settings| {
            let indicator = settings.indicators().get(at);
            let value = indicator.and_then(|indicator| indicator.inputs().get(number));
            value.copied().unwrap_or(input.default)
        })
    };
    let set = move |next: f64| {
        change(&tab, at, |indicator| {
            Some(indicator.with_input(number, next))
        });
    };
    let row = ui::label_edge_row(input.label);
    if input.whole() {
        let field = number_field(
            move || value() as u16,
            input.least as u16..=input.most as u16,
            input.step as u16,
            move |next| set(f64::from(next)),
        )
        .label(input.label)
        .hint(format!(
            "{} to {}",
            input.text(input.least),
            input.text(input.most)
        ))
        .test_id(&format!("chart-indicator-{}", input.id));
        row.child(field)
    } else {
        let select = ui::compact_dropdown(
            move || {
                input
                    .values()
                    .map(|value| {
                        let text = input.text(value);
                        SelectItem::new(text.clone(), text)
                    })
                    .collect()
            },
            move || SharedString::from(input.text(value())),
            move || input.text(value()),
            move |picked| {
                if let Ok(next) = picked.parse::<f64>() {
                    set(next);
                }
            },
            input.label,
        );
        row.child(select)
    }
}

/// "Color": the color the indicator `at` is drawn in.
fn color_row(tab: TabContext<ChartSettings>, at: usize) -> View {
    let saved = tab.settings;
    let color = move || {
        saved.with(|settings| {
            let indicator = settings.indicators().get(at);
            indicator.map_or(COLORS[0], Indicator::color)
        })
    };
    let select = ui::compact_dropdown(
        || {
            COLORS
                .iter()
                .map(|color| SelectItem::new(color.id(), color.label()))
                .collect()
        },
        move || SharedString::from(color().id()),
        move || color().label().to_owned(),
        move |id| {
            if let Some(color) = LinkColor::from_id(id) {
                change(&tab, at, |indicator| Some(indicator.with_color(color)));
            }
        },
        "Color",
    );
    ui::label_edge_row("Color").child(select)
}
