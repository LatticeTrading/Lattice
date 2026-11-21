//! Numbers as Lattice shows them, and decimal text compared exactly.
//!
//! Market data arrives as decimal strings (`"0.00005"`) or JSON numbers.
//! These helpers work on the text, so a price is never rounded through a
//! float on its way to the screen or into a sort.

use std::cmp::Ordering;

/// A price as the old app's GPU widgets wrote it (`formatPrice` of
/// `chart-gl/theme.ts`, which the chart and the order book shared): fewer
/// decimals the larger it is, `--` for one that is no number.
pub fn price_text(price: f64) -> String {
    if !price.is_finite() {
        return "--".to_owned();
    }
    let places = price_places(price);
    format!("{price:.places$}")
}

/// The decimals [`price_text`] writes a price of `price`'s size with, for
/// what is written beside prices in their own decimals (a difference
/// between two).
pub fn price_places(price: f64) -> usize {
    match price.abs() {
        size if size >= 10_000.0 => 1,
        size if size >= 1_000.0 => 2,
        size if size >= 1.0 => 3,
        _ => 5,
    }
}

/// An amount as the old order book wrote it (`formatCompactCount`):
/// millions with two decimals (`1.25M`), thousands with one (`19.3k`),
/// anything smaller as the nearest whole number.
pub fn compact_count(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        // Halves round up, as JavaScript's `Math.round`; never `-0`.
        let whole = (value + 0.5).floor();
        format!("{:.0}", if whole == 0.0 { 0.0 } else { whole })
    }
}

/// Unit letters of [`compact_amount`]: thousands up to quintillions.
const COMPACT_UNITS: [&str; 6] = ["k", "M", "B", "T", "P", "E"];

/// A plain decimal's sign, integer digits and fraction digits; `None` for
/// anything else (an exponent, spaces, no digits).
fn plain_decimal(text: &str) -> Option<(&str, &str, &str)> {
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'+' | b'-') => text.split_at(1),
        _ => ("", text),
    };
    let (whole, fraction) = rest.split_once('.').unwrap_or((rest, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    (!whole.is_empty() && digits(whole) && digits(fraction)).then_some((sign, whole, fraction))
}

/// Adds one to a string of decimal digits (`"199"` → `"200"`, `"99"` →
/// `"100"`).
fn increment(digits: &str) -> String {
    let mut out = digits.as_bytes().to_vec();
    for byte in out.iter_mut().rev() {
        if *byte == b'9' {
            *byte = b'0';
        } else {
            *byte += 1;
            return String::from_utf8(out).unwrap_or_default();
        }
    }
    let mut grown = String::with_capacity(out.len() + 1);
    grown.push('1');
    grown.push_str(std::str::from_utf8(&out).unwrap_or_default());
    grown
}

/// A decimal amount in a short form (the old app's `formatCompactAmount`):
/// three significant digits in the thousands, two from millions upward,
/// trailing zeros trimmed, half-up rounding (`"3456458.78"` → `"3.5M"`).
/// Amounts below 1000 and text that is not a plain decimal stay as given.
pub fn compact_amount(amount: &str) -> String {
    let Some((sign, whole, fraction)) = plain_decimal(amount) else {
        return amount.to_owned();
    };
    let integer_digits = whole.trim_start_matches('0').len();
    if integer_digits <= 3 {
        return amount.to_owned();
    }
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    let mut magnitude = integer_digits - 1;
    let mut unit = magnitude / 3;
    if unit > COMPACT_UNITS.len() {
        return amount.to_owned();
    }
    let significant = if unit == 1 { 3 } else { 2 };
    let mut mantissa = digits[..significant].to_owned();
    if digits
        .as_bytes()
        .get(significant)
        .is_some_and(|&d| d >= b'5')
    {
        mantissa = increment(&mantissa);
        if mantissa.len() > significant {
            mantissa = "1".to_owned();
            magnitude += 1;
            unit = magnitude / 3;
            if unit > COMPACT_UNITS.len() {
                return amount.to_owned();
            }
        }
    }
    let scaled = magnitude - unit * 3 + 1;
    let decimals = (if unit == 1 { 3_usize } else { 2 }).saturating_sub(scaled);
    let mut padded = mantissa;
    while padded.len() < scaled + decimals {
        padded.push('0');
    }
    let (integer, rest) = padded.split_at(scaled);
    let rest = rest.trim_end_matches('0');
    let unit = COMPACT_UNITS[unit - 1];
    if rest.is_empty() {
        format!("{sign}{integer}{unit}")
    } else {
        format!("{sign}{integer}.{rest}{unit}")
    }
}

/// A funding percentage for display (the old app's `formatFundingRate`):
/// at most five decimals, rounded half away from zero, no grouping, and
/// never `-0`.
pub fn funding_rate(value: &str) -> String {
    const PLACES: usize = 5;
    let Some((sign, whole, fraction)) = plain_decimal(value) else {
        return match value.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => funding_rate(&format!("{:.*}", PLACES + 1, number)),
            _ => value.to_owned(),
        };
    };
    let kept: String = fraction
        .chars()
        .chain(std::iter::repeat('0'))
        .take(PLACES)
        .collect();
    let mut digits = format!("{whole}{kept}");
    if fraction.as_bytes().get(PLACES).is_some_and(|&d| d >= b'5') {
        digits = increment(&digits);
    }
    let (integer, fraction) = digits.split_at(digits.len() - PLACES);
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let sign = if sign == "-" && (integer != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    }
}

/// A figure with `places` decimals and its thousands grouped
/// (`13,109.48`), `--` for one that is no number. One that rounds to zero
/// has no sign.
pub fn grouped(value: f64, places: usize) -> String {
    if !value.is_finite() {
        return "--".to_owned();
    }
    let digits = format!("{:.places$}", value.abs());
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    let mut out = String::with_capacity(digits.len() + whole.len() / 3 + 1);
    if value < 0.0 && digits.bytes().any(|digit| matches!(digit, b'1'..=b'9')) {
        out.push('-');
    }
    for (at, digit) in whole.chars().enumerate() {
        if at > 0 && (whole.len() - at) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// An amount of US dollars to the cent: `$13,109.48`, `-$100.01`.
pub fn usd(value: f64) -> String {
    let figure = grouped(value, 2);
    if !value.is_finite() {
        return figure;
    }
    match figure.strip_prefix('-') {
        Some(magnitude) => format!("-${magnitude}"),
        None => format!("${figure}"),
    }
}

/// The sizes [`compact_usd`] counts in, largest first, and their letters.
const USD_UNITS: [(f64, &str); 4] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")];

/// An amount of US dollars in a short form, four figures of it, for totals
/// too large to read to the cent (a protocol's value locked): `$7.151B`,
/// `$72.84M`, `$728.4K`, `$950`; `--` for one that is no number.
pub fn compact_usd(value: f64) -> String {
    if !value.is_finite() {
        return "--".to_owned();
    }
    let size = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    for (at, (unit, letter)) in USD_UNITS.iter().enumerate() {
        if size < *unit {
            continue;
        }
        let scaled = size / unit;
        let places = match scaled {
            scaled if scaled >= 100.0 => 1,
            scaled if scaled >= 10.0 => 2,
            _ => 3,
        };
        let figure = format!("{scaled:.places$}");
        // `999.96K` rounds up into the next size: `1.000M`.
        return match at.checked_sub(1).and_then(|larger| USD_UNITS.get(larger)) {
            Some((_, larger)) if figure.starts_with("1000") => format!("{sign}$1.000{larger}"),
            _ => format!("{sign}${figure}{letter}"),
        };
    }
    let whole = format!("{size:.0}");
    match whole.as_str() {
        "0" => "$0".to_owned(),
        "1000" => format!("{sign}$1.000K"),
        _ => format!("{sign}${whole}"),
    }
}

/// [`usd`] for what was gained or lost: a gain says so (`+$5.00`), and
/// nothing either way is `$0.00`.
pub fn signed_usd(value: f64) -> String {
    let amount = usd(value);
    if value > 0.0 && amount != "$0.00" {
        format!("+{amount}")
    } else {
        amount
    }
}

/// A fraction as a percentage with two decimals, a gain saying so:
/// `+1.25%`, `-0.27%`, `0.00%`.
pub fn signed_percent(fraction: f64) -> String {
    let figure = grouped(fraction * 100.0, 2);
    if !fraction.is_finite() {
        figure
    } else if fraction > 0.0 && figure != "0.00" {
        format!("+{figure}%")
    } else {
        format!("{figure}%")
    }
}

/// How much of something is held: fewer decimals the more there is (two
/// from a thousand, four from one, six below), without trailing zeros, its
/// thousands grouped (`2,000`, `14.6255`, `0.0335`).
pub fn quantity(value: f64) -> String {
    let places = match value.abs() {
        size if size >= 1_000.0 => 2,
        size if size >= 1.0 => 4,
        _ => 6,
    };
    let figure = grouped(value, places);
    if figure.contains('.') {
        figure
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        figure
    }
}

/// A decimal's sign, order of magnitude and significant digits, for
/// [`compare_decimals`].
fn magnitude(text: &str) -> (i8, i64, String) {
    let lower = text.trim().to_ascii_lowercase();
    let (mantissa, power) = lower.split_once('e').unwrap_or((&lower, "0"));
    let power: i64 = power.trim_start_matches('+').parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.trim_start_matches(['+', '-']);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let joined = format!("{whole}{fraction}");
    let raw = joined.trim_start_matches('0');
    let digits = raw.trim_end_matches('0');
    if digits.is_empty() {
        return (0, 0, String::new());
    }
    let order = power
        .saturating_add(raw.len() as i64)
        .saturating_sub(fraction.len() as i64);
    (if negative { -1 } else { 1 }, order, digits.to_owned())
}

/// Compares two decimal strings exactly, exponents included, without
/// rounding through a float (the old app's `compareScreenerDecimals`).
pub fn compare_decimals(left: &str, right: &str) -> Ordering {
    let (a_sign, a_order, a_digits) = magnitude(left);
    let (b_sign, b_order, b_digits) = magnitude(right);
    if a_sign != b_sign {
        return a_sign.cmp(&b_sign);
    }
    if a_sign == 0 {
        return Ordering::Equal;
    }
    let size = a_digits.len().max(b_digits.len());
    let pad = |digits: &str| format!("{digits:0<size$}");
    let unsigned = a_order
        .cmp(&b_order)
        .then_with(|| pad(&a_digits).cmp(&pad(&b_digits)));
    if a_sign < 0 {
        unsigned.reverse()
    } else {
        unsigned
    }
}

/// Text in a reader's order: letters regardless of case first, then case.
pub fn compare_text(left: &str, right: &str) -> Ordering {
    let fold = |text: &str| {
        text.chars()
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    fold(left).cmp(&fold(right)).then_with(|| left.cmp(right))
}

/// The date of the day `days` after 1970-01-01: its year, month (1 to 12)
/// and day of the month (Howard Hinnant's days-to-civil algorithm).
pub fn civil_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A Unix time in milliseconds as ISO 8601 UTC (`2026-11-12T08:30:00.000Z`,
/// what the old app's tooltips showed), or `unknown` without one or outside
/// years 0-9999.
pub fn iso_time(ms: Option<f64>) -> String {
    const DAY_MS: i64 = 86_400_000;
    let Some(ms) = ms.filter(|ms| ms.is_finite() && ms.abs() < 2.5e14) else {
        return "unknown".to_owned();
    };
    let ms = ms.floor() as i64;
    let (days, in_day) = (ms.div_euclid(DAY_MS), ms.rem_euclid(DAY_MS));
    let (year, month, day) = civil_date(days);
    if !(0..=9999).contains(&year) {
        return "unknown".to_owned();
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_dollar_amounts_are_written_short_in_four_figures() {
        assert_eq!(compact_usd(7_151_116_329.0), "$7.151B");
        assert_eq!(compact_usd(108_117_746_518.9), "$108.1B");
        assert_eq!(compact_usd(72_840_000.0), "$72.84M");
        assert_eq!(compact_usd(1_234_000_000_000.0), "$1.234T");
        assert_eq!(compact_usd(512_340.0), "$512.3K");
        assert_eq!(compact_usd(1_000.0), "$1.000K");
        assert_eq!(compact_usd(950.4), "$950");
        assert_eq!(compact_usd(0.0), "$0");
        assert_eq!(compact_usd(-0.2), "$0");
        assert_eq!(compact_usd(-2_500_000.0), "-$2.500M");
        // What rounds up into the next size is written in it.
        assert_eq!(compact_usd(999_960.0), "$1.000M");
        assert_eq!(compact_usd(999.6), "$1.000K");
        assert_eq!(compact_usd(999_999_999_999.0), "$1.000T");
        assert_eq!(compact_usd(f64::NAN), "--");
        assert_eq!(compact_usd(f64::INFINITY), "--");
    }

    #[test]
    fn figures_are_grouped_signed_and_rounded_as_they_show() {
        assert_eq!(grouped(13_109.482_328, 2), "13,109.48");
        assert_eq!(grouped(1_234_567.0, 0), "1,234,567");
        assert_eq!(grouped(999.995, 2), "1,000.00");
        assert_eq!(grouped(-1_000.5, 1), "-1,000.5");
        assert_eq!(grouped(12.0, 2), "12.00");
        // What rounds to zero has no sign; what is no number says so.
        assert_eq!(grouped(-0.004, 2), "0.00");
        assert_eq!(grouped(f64::NAN, 2), "--");
        assert_eq!(grouped(f64::INFINITY, 2), "--");

        assert_eq!(usd(13_109.482_328), "$13,109.48");
        assert_eq!(usd(-100.014), "-$100.01");
        assert_eq!(usd(0.0), "$0.00");
        assert_eq!(usd(f64::NAN), "--");

        assert_eq!(signed_usd(5.0), "+$5.00");
        assert_eq!(signed_usd(-5.0), "-$5.00");
        assert_eq!(signed_usd(0.0), "$0.00");
        assert_eq!(signed_usd(0.004), "$0.00");
        assert_eq!(signed_usd(-0.004), "$0.00");

        assert_eq!(signed_percent(0.0125), "+1.25%");
        assert_eq!(signed_percent(-0.002_678_9), "-0.27%");
        assert_eq!(signed_percent(0.0), "0.00%");
        assert_eq!(signed_percent(0.000_04), "0.00%");
        assert_eq!(signed_percent(f64::NAN), "--");
    }

    #[test]
    fn a_quantity_has_fewer_decimals_the_more_there_is() {
        assert_eq!(quantity(2_000.0), "2,000");
        assert_eq!(quantity(12_345.678), "12,345.68");
        assert_eq!(quantity(14.625_485), "14.6255");
        assert_eq!(quantity(20.0), "20");
        assert_eq!(quantity(0.0335), "0.0335");
        assert_eq!(quantity(0.000_012_3), "0.000012");
        assert_eq!(quantity(0.0), "0");
        assert_eq!(quantity(-0.1), "-0.1");
        assert_eq!(quantity(100.0), "100");
    }
}
