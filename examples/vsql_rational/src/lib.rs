#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use villagesql::{InValue, VdfReturn};

// ── Binary layout ─────────────────────────────────────────────────────────────
//
// 16 bytes: [numerator: i64 LE][denominator: i64 LE]
// Always stored in reduced form (GCD = 1) with a positive denominator.

const BYTES: usize = 16;

fn to_bytes(num: i64, den: i64) -> Vec<u8> {
    let mut v = Vec::with_capacity(BYTES);
    v.extend_from_slice(&num.to_le_bytes());
    v.extend_from_slice(&den.to_le_bytes());
    v
}

fn from_bytes(b: &[u8]) -> (i64, i64) {
    let num = i64::from_le_bytes(b[..8].try_into().unwrap());
    let den = i64::from_le_bytes(b[8..16].try_into().unwrap());
    (num, den)
}

// ── Arithmetic helpers ────────────────────────────────────────────────────────

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Reduce num/den to lowest terms with a positive denominator.
/// Returns None on zero denominator or i64 overflow after reduction.
fn normalize(num: i128, den: i128) -> Option<(i64, i64)> {
    if den == 0 {
        return None;
    }
    let sign: i128 = if den < 0 { -1 } else { 1 };
    let g = gcd(num.unsigned_abs(), den.unsigned_abs()).cast_signed();
    let n = sign * num / g;
    let d = sign * den / g;
    if n < i128::from(i64::MIN)
        || n > i128::from(i64::MAX)
        || d < i128::from(i64::MIN)
        || d > i128::from(i64::MAX)
    {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some((n as i64, d as i64))
}

// ── Type-system operations (encode / decode / compare / hash) ─────────────────

pub fn rational_encode(s: &str) -> Result<Vec<u8>, String> {
    let (num_s, den_s) = s
        .split_once('/')
        .ok_or_else(|| format!("rational: expected 'n/d', got {s:?}"))?;
    let num: i64 = num_s
        .trim()
        .parse()
        .map_err(|e| format!("rational numerator: {e}"))?;
    let den: i64 = den_s
        .trim()
        .parse()
        .map_err(|e| format!("rational denominator: {e}"))?;
    let (n, d) = normalize(i128::from(num), i128::from(den))
        .ok_or_else(|| "rational: zero or overflowing denominator".to_string())?;
    Ok(to_bytes(n, d))
}

pub fn rational_decode(b: &[u8]) -> Result<String, String> {
    if b.len() < BYTES {
        return Err(format!(
            "rational: expected {} bytes, got {}",
            BYTES,
            b.len()
        ));
    }
    let (n, d) = from_bytes(b);
    Ok(format!("{n}/{d}"))
}

#[must_use]
pub fn rational_compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    let (n1, d1) = from_bytes(a);
    let (n2, d2) = from_bytes(b);
    // n1/d1 vs n2/d2  →  cross-multiply (denominators are always positive)
    let lhs = i128::from(n1) * i128::from(d2);
    let rhs = i128::from(n2) * i128::from(d1);
    lhs.cmp(&rhs)
}

#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn rational_hash(b: &[u8]) -> usize {
    // FNV-1a over the 16 bytes
    let mut h: usize = 0xcbf2_9ce4_8422_2325_u64 as usize;
    for &byte in b {
        h ^= byte as usize;
        h = h.wrapping_mul(0x0100_0000_01b3_u64 as usize);
    }
    h
}

// ── Helper: decode an argument from the InValue slice ─────────────────────────

fn arg(args: &[InValue], pos: usize) -> Result<Option<(i64, i64)>, String> {
    match args.get(pos) {
        Some(InValue::Custom(b)) if b.len() >= BYTES => Ok(Some(from_bytes(b))),
        Some(InValue::Null) | None => Ok(None),
        Some(InValue::Custom(b)) => Err(format!("expected {} bytes, got {}", BYTES, b.len())),
        _ => Err("expected a rational argument".into()),
    }
}

// ── VDF implementations ───────────────────────────────────────────────────────

fn rational_add_impl(args: &[InValue]) -> VdfReturn {
    match (arg(args, 0), arg(args, 1)) {
        (Ok(Some((n1, d1))), Ok(Some((n2, d2)))) => {
            match normalize(
                i128::from(n1) * i128::from(d2) + i128::from(n2) * i128::from(d1),
                i128::from(d1) * i128::from(d2),
            ) {
                Some((n, d)) => VdfReturn::Binary(to_bytes(n, d)),
                None => VdfReturn::error("rational_add: overflow"),
            }
        }
        (Err(e), _) | (_, Err(e)) => VdfReturn::error(format!("rational_add: {e}")),
        _ => VdfReturn::null(),
    }
}

fn rational_sub_impl(args: &[InValue]) -> VdfReturn {
    match (arg(args, 0), arg(args, 1)) {
        (Ok(Some((n1, d1))), Ok(Some((n2, d2)))) => {
            match normalize(
                i128::from(n1) * i128::from(d2) - i128::from(n2) * i128::from(d1),
                i128::from(d1) * i128::from(d2),
            ) {
                Some((n, d)) => VdfReturn::Binary(to_bytes(n, d)),
                None => VdfReturn::error("rational_sub: overflow"),
            }
        }
        (Err(e), _) | (_, Err(e)) => VdfReturn::error(format!("rational_sub: {e}")),
        _ => VdfReturn::null(),
    }
}

fn rational_mul_impl(args: &[InValue]) -> VdfReturn {
    match (arg(args, 0), arg(args, 1)) {
        (Ok(Some((n1, d1))), Ok(Some((n2, d2)))) => {
            match normalize(
                i128::from(n1) * i128::from(n2),
                i128::from(d1) * i128::from(d2),
            ) {
                Some((n, d)) => VdfReturn::Binary(to_bytes(n, d)),
                None => VdfReturn::error("rational_mul: overflow"),
            }
        }
        (Err(e), _) | (_, Err(e)) => VdfReturn::error(format!("rational_mul: {e}")),
        _ => VdfReturn::null(),
    }
}

fn rational_div_impl(args: &[InValue]) -> VdfReturn {
    match (arg(args, 0), arg(args, 1)) {
        (Ok(Some((n1, d1))), Ok(Some((n2, d2)))) => {
            if n2 == 0 {
                return VdfReturn::error("rational_div: division by zero");
            }
            match normalize(
                i128::from(n1) * i128::from(d2),
                i128::from(d1) * i128::from(n2),
            ) {
                Some((n, d)) => VdfReturn::Binary(to_bytes(n, d)),
                None => VdfReturn::error("rational_div: overflow"),
            }
        }
        (Err(e), _) | (_, Err(e)) => VdfReturn::error(format!("rational_div: {e}")),
        _ => VdfReturn::null(),
    }
}

fn rational_numer_impl(args: &[InValue]) -> VdfReturn {
    match arg(args, 0) {
        Ok(Some((n, _))) => VdfReturn::Int(n),
        Ok(None) => VdfReturn::null(),
        Err(e) => VdfReturn::error(format!("rational_numer: {e}")),
    }
}

fn rational_denom_impl(args: &[InValue]) -> VdfReturn {
    match arg(args, 0) {
        Ok(Some((_, d))) => VdfReturn::Int(d),
        Ok(None) => VdfReturn::null(),
        Err(e) => VdfReturn::error(format!("rational_denom: {e}")),
    }
}

#[allow(clippy::cast_precision_loss)]
fn rational_to_real_impl(args: &[InValue]) -> VdfReturn {
    match arg(args, 0) {
        Ok(Some((n, d))) => VdfReturn::Real(n as f64 / d as f64),
        Ok(None) => VdfReturn::null(),
        Err(e) => VdfReturn::error(format!("rational_to_real: {e}")),
    }
}

// ── Extension registration ────────────────────────────────────────────────────

villagesql::extension! {
    funcs: [
        villagesql::func!(rational_add_impl, "rational_add",
            [villagesql::custom!("rational"), villagesql::custom!("rational")] -> villagesql::custom!("rational"),
            deterministic: true),
        villagesql::func!(rational_sub_impl, "rational_sub",
            [villagesql::custom!("rational"), villagesql::custom!("rational")] -> villagesql::custom!("rational"),
            deterministic: true),
        villagesql::func!(rational_mul_impl, "rational_mul",
            [villagesql::custom!("rational"), villagesql::custom!("rational")] -> villagesql::custom!("rational"),
            deterministic: true),
        villagesql::func!(rational_div_impl, "rational_div",
            [villagesql::custom!("rational"), villagesql::custom!("rational")] -> villagesql::custom!("rational"),
            deterministic: true),
        villagesql::func!(rational_numer_impl, "rational_numer",
            [villagesql::custom!("rational")] -> villagesql::Type::Int,
            deterministic: true),
        villagesql::func!(rational_denom_impl, "rational_denom",
            [villagesql::custom!("rational")] -> villagesql::Type::Int,
            deterministic: true),
        villagesql::func!(rational_to_real_impl, "rational_to_real",
            [villagesql::custom!("rational")] -> villagesql::Type::Real,
            deterministic: true),
    ],
    types: [
        villagesql::custom_type!(
            type_name: "rational",
            persisted_length: 16,
            max_decode_buffer_length: 42,
            encode: rational_encode,
            decode: rational_decode,
            compare: rational_compare,
            hash: rational_hash,
            default: "0/1",
        ),
    ]
}
