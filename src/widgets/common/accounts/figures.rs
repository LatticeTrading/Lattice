//! How the account widgets write and color their figures, so that a gain
//! reads the same in each: a figure an exchange does not report is a dash
//! ([`NONE`], [`shown`]), and what was gained or lost is green or red by
//! what shows of it ([`Tone`]).

use nsh_core::prelude::*;

use crate::theme;

/// What a figure that is not reported reads.
pub(crate) const NONE: &str = "—";

/// `figure` as `write` writes it; a dash when it is not reported.
pub(crate) fn shown(figure: Option<f64>, write: fn(f64) -> String) -> String {
    figure.map_or_else(|| NONE.to_owned(), write)
}

/// Which way a gain or loss reads: what colors it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Gain,
    Loss,
    Flat,
}

impl Tone {
    /// The tone of `amount` as it is shown, to the cent: one that rounds
    /// to nothing is flat.
    pub fn of(amount: f64) -> Self {
        let cents = (amount * 100.0).round();
        if cents > 0.0 {
            Self::Gain
        } else if cents < 0.0 {
            Self::Loss
        } else {
            Self::Flat
        }
    }

    /// The color a figure of `tone` is drawn in now (tracked); primary
    /// text for one with no tone (a figure that is not reported).
    pub fn color(tone: Option<Self>) -> AdaptiveColor {
        match tone {
            Some(Self::Gain) => theme::POSITIVE.get(),
            Some(Self::Loss) => theme::NEGATIVE.get(),
            Some(Self::Flat) => theme::TEXT_MUTED.get(),
            None => theme::TEXT.get(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format;

    #[test]
    fn a_figure_is_toned_by_what_shows_of_it() {
        assert_eq!(Tone::of(0.006), Tone::Gain);
        assert_eq!(Tone::of(12.5), Tone::Gain);
        assert_eq!(Tone::of(-0.006), Tone::Loss);
        // What rounds to nothing either way is flat.
        assert_eq!(Tone::of(0.004), Tone::Flat);
        assert_eq!(Tone::of(-0.004), Tone::Flat);
        assert_eq!(Tone::of(0.0), Tone::Flat);

        assert_eq!(shown(Some(1234.5), format::usd), "$1,234.50");
        assert_eq!(shown(None, format::usd), NONE);
    }
}
