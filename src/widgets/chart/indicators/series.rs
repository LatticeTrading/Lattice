//! An indicator's lines over a series ([`Line`]), and those of every
//! indicator of a chart ([`Computed`]): what the chart's session computes
//! from its bars when one of them or an indicator changes, which the
//! layer draws, the legend reads and the plot's fitted prices make room
//! for.

use super::Indicator;
use crate::data::ferris::Candle;

/// One line of an indicator over a series: a value a bar from bar `first`
/// on. The bars before it have none: too few came before them to compute
/// one from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    first: usize,
    values: Vec<f64>,
    /// Drawn fainter than the indicator's main line: a band about it.
    quiet: bool,
    /// The bars it starts over at, oldest first: each the first of a
    /// period whose values owe nothing to the one before, so the line is
    /// not joined to it from the bar before.
    restarts: Vec<usize>,
}

impl Line {
    /// The line of `values`, the first of them bar `first`'s.
    pub fn from(first: usize, values: Vec<f64>) -> Self {
        Self {
            first,
            values,
            quiet: false,
            restarts: Vec::new(),
        }
    }

    /// It starting over at each of the bars `at`, oldest first.
    pub fn restarting(mut self, at: Vec<usize>) -> Self {
        self.restarts = at;
        self
    }

    /// Whether it starts over after bar `from`, at bar `to` or before it:
    /// the two are not joined then.
    pub fn restarts(&self, from: usize, to: usize) -> bool {
        let next = self.restarts.partition_point(|bar| *bar <= from);
        self.restarts.get(next).is_some_and(|bar| *bar <= to)
    }

    /// It drawn fainter.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Its value at bar `bar`; none before its first value, past its last
    /// and where it is no number.
    pub fn at(&self, bar: usize) -> Option<f64> {
        let value = *self.values.get(bar.checked_sub(self.first)?)?;
        value.is_finite().then_some(value)
    }

    /// The lowest and highest it goes over bars `from..to`; none where it
    /// has no value.
    pub fn reach(&self, from: usize, to: usize) -> Option<(f64, f64)> {
        let from = from.saturating_sub(self.first);
        let to = to.saturating_sub(self.first).min(self.values.len());
        let values = self.values.get(from..to)?.iter();
        let reach = values
            .filter(|value| value.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                (low.min(*value), high.max(*value))
            });
        (reach.0 <= reach.1).then_some(reach)
    }
}

/// A chart's indicators over its series: each as it was computed with its
/// lines (none for a hidden one), in the settings' order, and how many
/// bars the series had.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Computed {
    bars: usize,
    all: Vec<(Indicator, Vec<Line>)>,
}

impl Computed {
    /// `indicators` over `bars` (oldest first).
    pub fn of(indicators: &[Indicator], bars: &[Candle]) -> Self {
        Self {
            bars: bars.len(),
            all: indicators
                .iter()
                .map(|indicator| (indicator.clone(), indicator.lines(bars)))
                .collect(),
        }
    }

    /// Whether the chart has no indicator.
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// Whether none of them has a line to draw: there is none, or each is
    /// hidden.
    pub fn is_blank(&self) -> bool {
        self.all.iter().all(|(_, lines)| lines.is_empty())
    }

    /// How many bars the series they were computed over had.
    pub fn bars(&self) -> usize {
        self.bars
    }

    /// Each indicator with its lines, in the settings' order.
    pub fn each(&self) -> impl Iterator<Item = (&Indicator, &[Line])> {
        self.all
            .iter()
            .map(|(indicator, lines)| (indicator, lines.as_slice()))
    }

    /// The lines of the settings' indicator `at`, which is `indicator`
    /// there; none until it is computed as it is now (the settings change
    /// a frame before it is).
    pub fn lines_of(&self, at: usize, indicator: &Indicator) -> Option<&[Line]> {
        let (computed, lines) = self.all.get(at)?;
        (computed == indicator).then_some(lines.as_slice())
    }

    /// The lowest and highest any of their lines goes over bars
    /// `from..to`; none where none has a value.
    pub fn reach(&self, from: usize, to: usize) -> Option<(f64, f64)> {
        let lines = self.all.iter().flat_map(|(_, lines)| lines);
        lines
            .filter_map(|line| line.reach(from, to))
            .reduce(|(low, high), (lowest, highest)| (low.min(lowest), high.max(highest)))
    }
}
