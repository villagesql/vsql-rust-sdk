//! Example `VillageSQL` extension: a variable-length custom type.
//!
//! `tag` is a short label: 1 to 64 bytes of UTF-8. It is declared with
//! `max_persisted_length:` instead of `persisted_length:`, so each value stores
//! only its own bytes. `'hi'` takes 2 bytes and `'hello'` takes 5.
//! `tag_size(tag)` returns how many bytes a value takes.
//!
//! A variable-length type needs VEF protocol 5 (`VillageSQL` 0.0.8 or later).

use std::cmp::Ordering;

use villagesql::{InValue, VdfReturn};

/// The most bytes a `tag` value can take. Matches `max_persisted_length` below.
const MAX_BYTES: usize = 64;

/// Text to stored bytes: the label's UTF-8, which must be 1 to 64 bytes.
fn tag_encode(s: &str) -> Result<Vec<u8>, String> {
    if s.is_empty() || s.len() > MAX_BYTES {
        return Err(format!(
            "tag: must be 1 to {MAX_BYTES} bytes, got {}",
            s.len()
        ));
    }
    Ok(s.as_bytes().to_vec())
}

/// Stored bytes back to text.
fn tag_decode(b: &[u8]) -> Result<String, String> {
    String::from_utf8(b.to_vec()).map_err(|e| e.to_string())
}

/// Byte order, which for UTF-8 is the same as code point order.
fn tag_compare(a: &[u8], b: &[u8]) -> Ordering {
    a.cmp(b)
}

/// SQL: `tag_size(tag)` -> INT: how many bytes the value takes in storage.
fn tag_size_impl(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::Custom(bytes)) => {
            VdfReturn::int(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
        }
        _ => VdfReturn::null(),
    }
}

villagesql::extension! {
    funcs: [
        villagesql::func!(tag_size_impl, "tag_size",
            [villagesql::custom!("tag")] -> villagesql::Type::Int,
            deterministic: true),
    ],
    types: [
        villagesql::custom_type!(
            type_name: "tag",
            max_persisted_length: 64,
            max_decode_buffer_length: 64,
            encode: tag_encode,
            decode: tag_decode,
            compare: tag_compare,
            default: "-",
        ),
    ]
}
