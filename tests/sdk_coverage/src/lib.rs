#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use vsql::{InValue, VdfReturn};

// ── Identity functions ────────────────────────────────────────────────────────

fn sdk_identity_int_impl(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::Int(v)) => VdfReturn::Int(*v),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("sdk_identity_int: expected INT"),
    }
}

fn sdk_identity_real_impl(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::Real(v)) => VdfReturn::Real(*v),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("sdk_identity_real: expected REAL"),
    }
}

// ── Warning function ──────────────────────────────────────────────────────────

fn sdk_warn_if_negative_impl(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::Int(v)) if *v < 0 => {
            VdfReturn::warning(format!("sdk_warn_if_negative: {v} is negative"))
        }
        Some(InValue::Int(v)) => VdfReturn::Int(*v),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("sdk_warn_if_negative: expected INT"),
    }
}

// ── Type 1: counter (wraps i64, decimal string representation) ────────────────

pub fn counter_encode(s: &str) -> Result<Vec<u8>, String> {
    let n: i64 = s.trim().parse().map_err(|e| format!("counter: {e}"))?;
    Ok(n.to_le_bytes().to_vec())
}

pub fn counter_decode(b: &[u8]) -> Result<String, String> {
    if b.len() < 8 {
        return Err(format!("counter: expected 8 bytes, got {}", b.len()));
    }
    Ok(i64::from_le_bytes(b[..8].try_into().unwrap()).to_string())
}

#[must_use]
pub fn counter_compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    let va = i64::from_le_bytes(a[..8].try_into().unwrap());
    let vb = i64::from_le_bytes(b[..8].try_into().unwrap());
    va.cmp(&vb)
}

#[must_use]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
pub fn counter_hash(b: &[u8]) -> usize {
    i64::from_le_bytes(b[..8].try_into().unwrap()) as usize
}

// ── Type 2: flag (1 byte bool, "true"/"false") ────────────────────────────────

pub fn flag_encode(s: &str) -> Result<Vec<u8>, String> {
    match s.trim() {
        "true" => Ok(vec![1]),
        "false" => Ok(vec![0]),
        other => Err(format!("flag: expected 'true' or 'false', got {other:?}")),
    }
}

pub fn flag_decode(b: &[u8]) -> Result<String, String> {
    if b.is_empty() {
        return Err("flag: expected 1 byte, got 0".to_string());
    }
    Ok(if b[0] != 0 { "true" } else { "false" }.to_string())
}

#[must_use]
pub fn flag_compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a[0].cmp(&b[0])
}

// ── Extension registration ────────────────────────────────────────────────────

vsql::extension! {
    funcs: [
        vsql::func!(sdk_identity_int_impl, "sdk_identity_int",
            [vsql::Type::Int] -> vsql::Type::Int),
        vsql::func!(sdk_identity_real_impl, "sdk_identity_real",
            [vsql::Type::Real] -> vsql::Type::Real),
        vsql::func!(sdk_warn_if_negative_impl, "sdk_warn_if_negative",
            [vsql::Type::Int] -> vsql::Type::Int),
    ],
    types: [
        vsql::custom_type!(
            type_name: "counter",
            persisted_length: 8,
            max_decode_buffer_length: 20,
            encode: counter_encode,
            decode: counter_decode,
            compare: counter_compare,
            hash: counter_hash,
            default: "0",
        ),
        vsql::custom_type!(
            type_name: "flag",
            persisted_length: 1,
            max_decode_buffer_length: 5,
            encode: flag_encode,
            decode: flag_decode,
            compare: flag_compare,
            default: "false",
        ),
    ]
}
