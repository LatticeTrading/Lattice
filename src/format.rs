//! Numbers as Lattice shows them, and decimal text compared exactly.
//!
//! Market data arrives as decimal strings (`"0.00005"`) or JSON numbers.
//! These helpers work on the text, so a price is never rounded through a
//! float on its way to the screen or into a sort.

use std::cmp::Ordering;

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
