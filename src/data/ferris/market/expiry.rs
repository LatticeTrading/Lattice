//! Date and symbol text helpers: contract expiries and their labels, ISO
//! and compact days, settlement suffixes and plain pairs.

/// Month names as expiry labels and native ids spell them.
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// A contract's expiry, `2026-12-25` or `2026-12` (the old app's
/// `deriveMarketExpiry`): the catalog's own when it reads as a date, else
/// from the native id (`BTCUSDT_261225`, `BTCUSDT-04DEC26`, or a month code
/// `BTCUSDH27`, which has no day).
pub(super) fn derive_expiry(exchange_symbol: &str, explicit: Option<&str>) -> Option<String> {
    const CODES: &str = "FGHJKMNQUVXZ";
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if let Some(explicit) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        let year = explicit.get(..4).filter(|y| digits(y));
        let month = explicit
            .get(4..7)
            .and_then(|m| m.strip_prefix('-'))
            .filter(|m| digits(m));
        if let (Some(year), Some(month)) = (year, month) {
            let day = explicit
                .get(7..10)
                .and_then(|d| d.strip_prefix('-'))
                .filter(|d| digits(d));
            return Some(match day {
                Some(day) => format!("{year}-{month}-{day}"),
                None => format!("{year}-{month}"),
            });
        }
    }
    let native = exchange_symbol.trim().to_uppercase();
    let len = native.len();
    // `_YYMMDD`
    if let Some(compact) = native.get(len.saturating_sub(7)..)
        && let Some(compact) = compact.strip_prefix('_')
        && digits(compact)
    {
        return Some(format!(
            "20{}-{}-{}",
            &compact[..2],
            &compact[2..4],
            &compact[4..]
        ));
    }
    // `-DDMMMYY`
    if let Some(named) = native.get(len.saturating_sub(8)..)
        && let Some(named) = named.strip_prefix('-')
        && let (Some(day), Some(name), Some(year)) =
            (named.get(..2), named.get(2..5), named.get(5..))
        && digits(day)
        && digits(year)
        && let Some(month) = MONTHS.iter().position(|m| *m == name)
    {
        return Some(format!("20{year}-{:02}-{day}", month + 1));
    }
    // A month code and two digits of year: no day.
    if let Some(coded) = native.get(len.saturating_sub(3)..)
        && let (Some(code), Some(year)) = (coded.chars().next(), coded.get(1..))
        && digits(year)
        && let Some(month) = CODES.find(code)
    {
        return Some(format!("20{year}-{:02}", month + 1));
    }
    None
}

/// Text with no `/`, `:` or whitespace, and some of it.
pub(super) fn is_bare(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c == '/' || c == ':' || c.is_whitespace())
}

/// `BASE/QUOTE` with both parts bare.
pub(super) fn plain_pair(pair: &str) -> Option<(&str, &str)> {
    let (base, quote) = pair.split_once('/')?;
    (is_bare(base) && is_bare(quote)).then_some((base, quote))
}

/// `2026-12-25`.
pub(super) fn is_iso_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(at, byte)| at == 4 || at == 7 || byte.is_ascii_digit())
}

/// `2026-12-25` as `261225`.
pub(super) fn compact_day(day: &str) -> String {
    day.get(2..)
        .unwrap_or_default()
        .chars()
        .filter(|c| *c != '-')
        .collect()
}

/// The settlement after a request symbol's `:` (`BTC/USDT:USDT-261225`
/// settles in `USDT`).
pub(super) fn settlement_of(request_symbol: &str) -> Option<String> {
    let (_, after) = request_symbol.split_once(':')?;
    let settlement: String = after
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let rest = &after[settlement.len()..];
    (!settlement.is_empty() && (rest.is_empty() || rest.starts_with('-')))
        .then(|| settlement.to_ascii_uppercase())
}

/// `2026-12-25` as `25 DEC 2026`, `2026-12` as `DEC 2026`.
pub(super) fn expiry_label(expiry: &str) -> Option<String> {
    let mut parts = expiry.split('-');
    let year = parts.next().filter(|y| y.len() == 4)?;
    let month: usize = parts.next().filter(|m| m.len() == 2)?.parse().ok()?;
    let name = MONTHS.get(month.checked_sub(1)?)?;
    match parts.next() {
        None => Some(format!("{name} {year}")),
        Some(day) if day.len() == 2 && parts.next().is_none() => {
            day.parse::<u8>().ok()?;
            Some(format!("{day} {name} {year}"))
        }
        Some(_) => None,
    }
}
