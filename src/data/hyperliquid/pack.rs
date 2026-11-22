//! An action in its two forms: posted as JSON, and hashed as MessagePack
//! for the signature that authorizes it
//! ([`super::signing::action_hash`]). Hyperliquid reads the JSON into its
//! own shape and hashes that again, its members in its own order, to see
//! who signed. So what is hashed here must hold the same members in
//! exactly that order (its SDK's), or the signature is somebody else's as
//! far as Hyperliquid can tell. An action is made once, as a [`Packed`]
//! value whose maps keep the order they were written in, and both forms
//! are written from it.
//!
//! The MessagePack is the smallest form of each value, which is what
//! Hyperliquid's own SDK hashes (`msgpack.packb`) and its Rust ones
//! (`rmp_serde::to_vec_named`). Actions hold text, whole numbers, booleans,
//! lists and maps; a price or a size is decimal text
//! ([`super::exchange::decimal`]), never a float. That is all there is
//! here, and why no MessagePack crate is: the format of these six shapes
//! is forty lines.

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};

/// A value of an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Packed {
    Bool(bool),
    /// A whole number: an asset's index, an order's id, a time.
    Uint(u64),
    Text(String),
    List(Vec<Self>),
    /// Members in the order Hyperliquid's SDK writes them, which is the
    /// order they are hashed in.
    Map(Vec<(&'static str, Self)>),
}

impl Packed {
    pub(super) fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// Appends the value's MessagePack to `out`.
    pub(super) fn pack(&self, out: &mut Vec<u8>) {
        match self {
            Self::Bool(false) => out.push(0xc2),
            Self::Bool(true) => out.push(0xc3),
            Self::Uint(number) => {
                let bytes = number.to_be_bytes();
                match *number {
                    0..=0x7f => out.push(bytes[7]),
                    0x80..=0xff => out.extend([0xcc, bytes[7]]),
                    0x100..=0xffff => {
                        out.push(0xcd);
                        out.extend(&bytes[6..]);
                    }
                    0x1_0000..=0xffff_ffff => {
                        out.push(0xce);
                        out.extend(&bytes[4..]);
                    }
                    _ => {
                        out.push(0xcf);
                        out.extend(bytes);
                    }
                }
            }
            Self::Text(text) => {
                head(out, text.len(), 31, [0xa0, 0xd9, 0xda, 0xdb]);
                out.extend(text.as_bytes());
            }
            Self::List(items) => {
                head(out, items.len(), 15, [0x90, 0, 0xdc, 0xdd]);
                for item in items {
                    item.pack(out);
                }
            }
            Self::Map(members) => {
                head(out, members.len(), 15, [0x80, 0, 0xde, 0xdf]);
                for (name, member) in members {
                    head(out, name.len(), 31, [0xa0, 0xd9, 0xda, 0xdb]);
                    out.extend(name.as_bytes());
                    member.pack(out);
                }
            }
        }
    }
}

/// Writes the head of a value `len` long: up to `short` it is one byte,
/// `marks[0]` with the length in it; a longer one is the mark of the
/// narrowest length that holds it (`marks[1]` one byte, which lists and
/// maps have no form of and name `0`; `marks[2]` two; `marks[3]` four),
/// then the length.
fn head(out: &mut Vec<u8>, len: usize, short: usize, marks: [u8; 4]) {
    // A length is never wider than 64 bits; nothing an action holds is four
    // gigabytes long.
    let bytes = (len as u64).to_be_bytes();
    if len <= short {
        out.push(marks[0] | bytes[7]);
    } else if len <= 0xff && marks[1] != 0 {
        out.extend([marks[1], bytes[7]]);
    } else if len <= 0xffff {
        out.push(marks[2]);
        out.extend(&bytes[6..]);
    } else {
        out.push(marks[3]);
        out.extend(&bytes[4..]);
    }
}

/// As JSON, members in the order written.
impl Serialize for Packed {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Bool(flag) => serializer.serialize_bool(*flag),
            Self::Uint(number) => serializer.serialize_u64(*number),
            Self::Text(text) => serializer.serialize_str(text),
            Self::List(items) => {
                let mut list = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    list.serialize_element(item)?;
                }
                list.end()
            }
            Self::Map(members) => {
                let mut map = serializer.serialize_map(Some(members.len()))?;
                for (name, member) in members {
                    map.serialize_entry(name, member)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(value: &Packed) -> Vec<u8> {
        let mut out = Vec::new();
        value.pack(&mut out);
        out
    }

    #[test]
    fn values_pack_in_their_smallest_form() {
        assert_eq!(packed(&Packed::Bool(false)), [0xc2]);
        assert_eq!(packed(&Packed::Bool(true)), [0xc3]);
        // Each width of a whole number, at both of its ends.
        for (number, bytes) in [
            (0, vec![0x00]),
            (127, vec![0x7f]),
            (128, vec![0xcc, 0x80]),
            (255, vec![0xcc, 0xff]),
            (256, vec![0xcd, 0x01, 0x00]),
            (65_535, vec![0xcd, 0xff, 0xff]),
            (65_536, vec![0xce, 0x00, 0x01, 0x00, 0x00]),
            (4_294_967_295, vec![0xce, 0xff, 0xff, 0xff, 0xff]),
            (
                4_294_967_296,
                vec![0xcf, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
            ),
        ] {
            assert_eq!(packed(&Packed::Uint(number)), bytes, "{number}");
        }
        // Text by its length in bytes: 31 fit the head, then a byte of
        // length, then two.
        assert_eq!(packed(&Packed::text("")), [0xa0]);
        assert_eq!(packed(&Packed::text("Gtc")), [0xa3, b'G', b't', b'c']);
        assert_eq!(packed(&Packed::text("a".repeat(31)))[..1], [0xbf]);
        assert_eq!(packed(&Packed::text("a".repeat(32)))[..2], [0xd9, 32]);
        assert_eq!(
            packed(&Packed::text("a".repeat(256)))[..3],
            [0xda, 0x01, 0x00]
        );
        // Lists and maps have no one-byte length: 16 members take two.
        let list = |len: usize| Packed::List(vec![Packed::Uint(1); len]);
        assert_eq!(packed(&list(0)), [0x90]);
        assert_eq!(packed(&list(15))[..1], [0x9f]);
        assert_eq!(packed(&list(16))[..3], [0xdc, 0x00, 0x10]);
        let map = Packed::Map(vec![("a", Packed::Uint(1)), ("b", Packed::Bool(true))]);
        assert_eq!(packed(&map), [0x82, 0xa1, b'a', 0x01, 0xa1, b'b', 0xc3]);
    }

    #[test]
    fn the_json_keeps_the_members_order() {
        let action = Packed::Map(vec![
            ("type", Packed::text("order")),
            (
                "orders",
                Packed::List(vec![Packed::Map(vec![
                    ("a", Packed::Uint(1)),
                    ("b", Packed::Bool(true)),
                ])]),
            ),
            ("grouping", Packed::text("na")),
        ]);
        assert_eq!(
            serde_json::to_string(&action).unwrap(),
            r#"{"type":"order","orders":[{"a":1,"b":true}],"grouping":"na"}"#
        );
    }
}
