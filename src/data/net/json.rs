//! JSON as APIs write it: stable keys ([`canonical_json`]), so two requests
//! or topics that mean the same thing share one key whatever order their
//! members were written in, and figures written as decimal text
//! ([`Figure`]).

use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::Value;

/// A figure as an API writes it: a JSON number, or decimal text
/// (`"13104.514502"`, how exchanges keep a float from rounding it on the
/// way). For what is added up, plotted or shown rounded (an account's
/// value, a position's size); a price shown digit for digit stays text.
/// Anything else, or a number that is not finite, fails to read.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Figure(pub f64);

impl Figure {
    /// The figure `value` holds, if it holds one.
    pub fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
        .filter(|number: &f64| number.is_finite())
        .map(Self)
    }
}

impl<'de> Deserialize<'de> for Figure {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::of(&value).ok_or_else(|| D::Error::custom(format!("`{value}` is not a number")))
    }
}

/// `value` as compact JSON with every object's members sorted by name, at
/// every depth.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut members: Vec<_> = map.iter().collect();
            members.sort_unstable_by_key(|(name, _)| *name);
            out.push('{');
            for (index, (name, member)) in members.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(&Value::String(name.clone()), out);
                out.push(':');
                write(member, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        // Scalars have one JSON form; serde_json's escaping is canonical.
        scalar => out.push_str(&scalar.to_string()),
    }
}
