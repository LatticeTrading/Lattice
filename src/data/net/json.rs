//! Stable keys from JSON, so two requests or topics that mean the same
//! thing share one key whatever order their members were written in.

use serde_json::Value;

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
