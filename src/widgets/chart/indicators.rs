//! What the chart computes from its bars and draws over them
//! ([`Indicator`]): an exponential moving average, a triple exponential
//! one and Bollinger Bands, each added from the menu that the badge beside
//! the plot's header opens.
//!
//! An indicator is a kind ([`Kind`]), the numbers it is computed with (its
//! kind's [`Input`]s: a length, a multiplier), a color and whether it
//! shows. It is the chart's, not a market's: it is computed from the
//! closes of whatever the chart charts, candles or ticks. Indicators are
//! saved in the chart's settings (`indicators`) in the order they were
//! added, which is the order the legend lists them in and the layer draws
//! them in (later ones over earlier ones).
//!
//! Its parts:
//!
//! - [`series`]: an indicator's lines over a series ([`Line`]) and every
//!   indicator's ([`Computed`]), which the chart's session computes when
//!   a bar or an indicator changes;
//! - one module a kind ([`ema`], [`tema`], [`bands`]): what it computes;
//! - [`layer`]: the canvas over the plot the lines are painted on;
//! - [`legend`]: a row an indicator under the plot's header (its name, its
//!   values at the bar under the pointer, and its controls: hiding it,
//!   its settings, removing it);
//! - [`panel`]: an indicator's settings (its inputs and its color);
//! - [`menu`]: the badge beside the header, and the menu of kinds it
//!   opens.
//!
//! A new kind is its module with what it computes and its line in [`Kind`]
//! (its id, its names, its inputs, its lines). The menu, the legend, the
//! settings, the layer and the chart's settings take it from there.

mod bands;
mod ema;
mod layer;
mod legend;
mod menu;
mod panel;
mod series;
mod tema;

use nsh_core::{element::ImageSrc, prelude::*};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;

#[cfg(test)]
pub(crate) use self::layer::{Ink, paint};
pub(crate) use self::{
    layer::layer,
    legend::{legend, legend_height},
    menu::{BADGE_WIDTH, badge},
    series::{Computed, Line},
};
use super::ChartSettings;
use crate::{data::ferris::Candle, link::LinkColor, widgets::TabContext};

/// The colors an indicator can be drawn in, in the order new ones take
/// them: the theme's five that stay told apart (the link colors'), the
/// ones candles are seldom drawn in first.
pub(crate) const COLORS: [LinkColor; 5] = [
    LinkColor::Yellow,
    LinkColor::Blue,
    LinkColor::Orange,
    LinkColor::Green,
    LinkColor::Red,
];

/// A number an indicator is computed with (its length, a multiplier): what
/// it is called, what it starts as, and the values it takes, every `step`
/// from `least` to `most`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Input {
    /// What settings save it as, and what its field's test id ends with.
    pub id: &'static str,
    pub label: &'static str,
    pub default: f64,
    pub least: f64,
    pub most: f64,
    pub step: f64,
}

impl Input {
    /// How many bars a value is computed from.
    const LENGTH: Self = Self {
        id: "length",
        label: "Length",
        default: 20.0,
        least: 1.0,
        most: 500.0,
        step: 1.0,
    };

    /// How many standard deviations a band is from its basis.
    const DEVIATIONS: Self = Self {
        id: "mult",
        label: "StdDev",
        default: 2.0,
        least: 0.5,
        most: 5.0,
        step: 0.5,
    };

    /// `value` as one it takes: the nearest of its steps within its range,
    /// and what it starts as for what is no number.
    pub fn fit(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return self.default;
        }
        let steps = ((value.clamp(self.least, self.most) - self.least) / self.step).round();
        (self.least + steps * self.step).min(self.most)
    }

    /// Whether its values are whole numbers: typed in a number field, where
    /// another's are picked from a list.
    pub fn whole(&self) -> bool {
        self.least.fract() == 0.0 && self.step.fract() == 0.0
    }

    /// Every value it takes, the least first.
    pub fn values(&self) -> impl Iterator<Item = f64> {
        let (least, step) = (self.least, self.step);
        let steps = ((self.most - least) / step).floor() as usize;
        (0..=steps).map(move |at| least + at as f64 * step)
    }

    /// A value of it as it is written (`20`, `2.5`).
    pub fn text(&self, value: f64) -> String {
        let written = format!("{value:.2}");
        let written = written.trim_end_matches('0').trim_end_matches('.');
        written.to_owned()
    }
}

/// The kinds of indicator, in the order the menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// The closes' exponential moving average.
    Ema,
    /// Their triple exponential moving average, which lags them less.
    Tema,
    /// Bollinger Bands: their simple moving average, and a band above and
    /// below it some standard deviations away.
    Bands,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Ema, Self::Tema, Self::Bands];

    /// What settings save it as, and what its menu row's test id ends
    /// with.
    pub fn id(self) -> &'static str {
        match self {
            Self::Ema => "ema",
            Self::Tema => "tema",
            Self::Bands => "bb",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Its short name, which starts its row of the legend (`EMA 20`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Ema => "EMA",
            Self::Tema => "TEMA",
            Self::Bands => "BB",
        }
    }

    /// Its whole name, which the menu lists it by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ema => "Exponential Moving Average",
            Self::Tema => "Triple Exponential Moving Average",
            Self::Bands => "Bollinger Bands",
        }
    }

    /// What one is computed with, in the order its row names their values
    /// and its settings list them. Every kind's first is its length.
    pub fn inputs(self) -> &'static [Input] {
        match self {
            Self::Ema | Self::Tema => &[Input::LENGTH],
            Self::Bands => &[Input::LENGTH, Input::DEVIATIONS],
        }
    }
}

/// One indicator on a chart: its kind, a value an input of its kind (in
/// their order), its color, and whether it is hidden. Its values are
/// numbers its inputs take ([`Input::fit`] makes no other), so two
/// indicators are equal or they are not.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Indicator {
    kind: Kind,
    inputs: Vec<f64>,
    color: LinkColor,
    hidden: bool,
}

impl Eq for Indicator {}

impl Indicator {
    /// A `kind` as one starts: its inputs at what they start as, showing,
    /// in the first of [`COLORS`] none of `others` (the chart's other
    /// indicators) is drawn in, or once they are all taken the one after
    /// the last's.
    pub fn new(kind: Kind, others: &[Self]) -> Self {
        let taken = |color: &LinkColor| others.iter().any(|other| other.color == *color);
        let color = COLORS
            .into_iter()
            .find(|color| !taken(color))
            .unwrap_or(COLORS[others.len() % COLORS.len()]);
        Self {
            kind,
            inputs: kind.inputs().iter().map(|input| input.default).collect(),
            color,
            hidden: false,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// A value an input of its kind, in their order.
    pub fn inputs(&self) -> &[f64] {
        &self.inputs
    }

    pub fn color(&self) -> LinkColor {
        self.color
    }

    /// Whether it is kept off the plot: it stays in the legend.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// What the legend calls it: its kind and its inputs (`EMA 20`,
    /// `BB 20 2`).
    pub fn title(&self) -> String {
        let inputs = self.kind.inputs().iter().zip(&self.inputs);
        inputs.fold(self.kind.label().to_owned(), |title, (input, value)| {
            format!("{title} {}", input.text(*value))
        })
    }

    /// It with the input `at` of its kind at `value`, as the input takes
    /// it.
    pub fn with_input(mut self, at: usize, value: f64) -> Self {
        if let (Some(input), Some(held)) = (self.kind.inputs().get(at), self.inputs.get_mut(at)) {
            *held = input.fit(value);
        }
        self
    }

    pub fn with_color(mut self, color: LinkColor) -> Self {
        self.color = color;
        self
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Its lines over `bars` (oldest first); none while it is hidden.
    pub fn lines(&self, bars: &[Candle]) -> Vec<Line> {
        if self.hidden {
            return Vec::new();
        }
        let closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
        // One value an input, whole where the input is: `new`, `read` and
        // `with_input` make no other.
        let length = self.inputs[0] as usize;
        match self.kind {
            Kind::Ema => vec![ema::line(&closes, length)],
            Kind::Tema => vec![tema::line(&closes, length)],
            Kind::Bands => bands::lines(&closes, length, self.inputs[1]),
        }
    }

    /// The indicator saved as `saved`; none for a kind Lattice does not
    /// know. An input that is missing or no number is what it starts as,
    /// one its input does not take the nearest it does, and a color
    /// Lattice does not know the first.
    fn read(saved: &Value) -> Option<Self> {
        let kind = Kind::from_id(saved.get("type")?.as_str()?)?;
        let number = |input: &Input| saved.get(input.id).and_then(Value::as_f64);
        let color = saved.get("color").and_then(Value::as_str);
        Some(Self {
            kind,
            inputs: kind
                .inputs()
                .iter()
                .map(|input| number(input).map_or(input.default, |value| input.fit(value)))
                .collect(),
            color: color.and_then(LinkColor::from_id).unwrap_or(COLORS[0]),
            hidden: saved.get("hidden").and_then(Value::as_bool) == Some(true),
        })
    }

    /// The indicators saved as the list `saved`, leaving out what is none.
    pub fn read_all(saved: Option<&Value>) -> Vec<Self> {
        let saved = saved.and_then(Value::as_array);
        saved.map_or_else(Vec::new, |all| all.iter().filter_map(Self::read).collect())
    }
}

/// Saved as its kind (`type`), each input under its id (a whole one as a
/// whole number), its color's id, and `hidden` while it is.
impl Serialize for Indicator {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut saved = serializer.serialize_map(None)?;
        saved.serialize_entry("type", self.kind.id())?;
        for (input, value) in self.kind.inputs().iter().zip(&self.inputs) {
            if input.whole() {
                saved.serialize_entry(input.id, &(*value as u64))?;
            } else {
                saved.serialize_entry(input.id, value)?;
            }
        }
        saved.serialize_entry("color", self.color.id())?;
        if self.hidden {
            saved.serialize_entry("hidden", &true)?;
        }
        saved.end()
    }
}

/// Saves the chart of `tab` with its indicator `at` as `change` makes it,
/// or without it when `change` makes none.
fn change(
    tab: &TabContext<ChartSettings>,
    at: usize,
    change: impl FnOnce(Indicator) -> Option<Indicator>,
) {
    let settings = tab.settings.get_untracked();
    let mut indicators = settings.indicators().to_vec();
    let Some(held) = indicators.get(at).cloned() else {
        return;
    };
    match change(held) {
        Some(next) => indicators[at] = next,
        None => {
            indicators.remove(at);
        }
    }
    tab.save(&settings.with_indicators(indicators));
}

/// The icon `markup` gives (an SVG's, tracked), `side` points across.
fn icon(markup: impl Fn() -> &'static str + 'static, side: f32) -> View {
    view()
        .size(px(side))
        .flex_none()
        .child(svg(move || ImageSrc::svg_markup(markup())).size_full())
}
