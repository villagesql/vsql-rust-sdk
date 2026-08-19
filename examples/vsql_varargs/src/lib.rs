#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt::Write as _;
use villagesql::{InValue, PrerunArgs, PrerunResult, VdfReturn};

/// Per-statement state: how many times the row handler has run this statement.
/// Proves state and varargs work together (mirrors the C++ `CallCounter`).
#[derive(Default)]
struct JoinState {
    calls: i64,
}

/// Validate the call and set up the statement. The server does no validation for
/// varargs, so this is the only gate.
fn str_join_prerun(args: PrerunArgs, mut out: PrerunResult<JoinState>) {
    // Reject a zero-argument call.
    if args.is_empty() {
        out.error("str_join requires at least one argument");
        return;
    }

    // Every argument must be a string.
    for i in 0..args.len() {
        if !args.type_at(i).is_some_and(|t| t.is_str()) {
            out.error("str_join: every argument must be a string");
            return;
        }
    }

    // Size the result buffer from the arg count.
    out.request_buffer_size(32 + args.len() * 64);

    // Hand the fresh counter to the server.
    out.set_state(JoinState::default());
}

/// Join a variable number of string arguments, prefixed with the per-statement
/// call count. `args` length varies with how many arguments the SQL call passed.
fn str_join(state: &mut JoinState, args: &[InValue]) -> VdfReturn {
    state.calls += 1;

    let mut joined = String::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            InValue::String(s) => {
                if i > 0 {
                    joined.push_str(", ");
                }
                joined.push_str(s);
            }
            // A string column can carry NULL. SQL-style: NULL in -> NULL out.
            InValue::Null => return VdfReturn::Null,
            _ => return VdfReturn::error("str_join: non-string argument at runtime"),
        }
    }
    VdfReturn::string(format!("#{}: {joined}", state.calls))
}

/// Bare varargs: no prerun, no state, no validation. Returns how many arguments it
/// was called with, including zero, which the validated `str_join` would reject.
fn arg_count(args: &[InValue]) -> VdfReturn {
    VdfReturn::int(i64::try_from(args.len()).unwrap_or(i64::MAX))
}

/// Prerun-only varargs: validates but keeps no state. Accepts a mix of INT / REAL /
/// STRING arguments (heterogeneous is fine) and rejects custom-typed args.
fn describe_prerun(args: PrerunArgs, mut out: PrerunResult<()>) {
    if args.is_empty() {
        out.error("describe requires at least one argument");
        return;
    }
    for i in 0..args.len() {
        let ok = args
            .type_at(i)
            .is_some_and(|t| t.is_int() || t.is_real() || t.is_str());
        if !ok {
            out.error("describe: arguments must be INT, REAL, or STRING");
            return;
        }
    }
    out.request_buffer_size(32 + args.len() * 48);
}

/// Describe each argument as "type:value", joined with ", ". Demonstrates
/// per-argument type handling on a heterogeneous varargs call.
fn describe(args: &[InValue]) -> VdfReturn {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match arg {
            InValue::Int(n) => write!(out, "int:{n}").unwrap(),
            InValue::Real(r) => write!(out, "real:{r}").unwrap(),
            InValue::String(s) => write!(out, "str:{s}").unwrap(),
            InValue::Null => out.push_str("null"),
            _ => return VdfReturn::error("describe: unexpected non-scalar argument"),
        }
    }
    VdfReturn::string(out)
}

// point2d binary layout: 8 bytes = [x: i32 LE][y: i32 LE]
const POINT_BYTES: usize = 8;

fn point_to_bytes(x: i32, y: i32) -> Vec<u8> {
    let mut v = Vec::with_capacity(POINT_BYTES);
    v.extend_from_slice(&x.to_le_bytes());
    v.extend_from_slice(&y.to_le_bytes());
    v
}

fn point_from_bytes(b: &[u8]) -> (i32, i32) {
    let x = i32::from_le_bytes(b[..4].try_into().unwrap());
    let y = i32::from_le_bytes(b[4..8].try_into().unwrap());
    (x, y)
}

pub fn point_encode(s: &str) -> Result<Vec<u8>, String> {
    let (xs, ys) = s
        .split_once(',')
        .ok_or_else(|| format!("point2d: expected 'x,y', got {s:?}"))?;
    let x: i32 = xs.trim().parse().map_err(|e| format!("point2d x: {e}"))?;
    let y: i32 = ys.trim().parse().map_err(|e| format!("point2d y: {e}"))?;
    Ok(point_to_bytes(x, y))
}

pub fn point_decode(b: &[u8]) -> Result<String, String> {
    if b.len() < POINT_BYTES {
        return Err(format!(
            "point2d: expected {POINT_BYTES} bytes, got {}",
            b.len()
        ));
    }
    let (x, y) = point_from_bytes(b);
    Ok(format!("{x},{y}"))
}

#[must_use]
pub fn point_compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    point_from_bytes(a).cmp(&point_from_bytes(b))
}

/// Validate that every argument is a point2d. This is the `is_custom`/`custom_name`
/// path.
fn point_path_prerun(args: PrerunArgs, mut out: PrerunResult<()>) {
    if args.is_empty() {
        out.error("point_path requires at least one point");
        return;
    }
    for i in 0..args.len() {
        let ok = args
            .type_at(i)
            .is_some_and(|t| t.is_custom() && t.custom_name() == Some("point2d"));
        if !ok {
            out.error("point_path: every argument must be a point2d");
            return;
        }
    }
    out.request_buffer_size(16 + args.len() * 32);
}

/// Join a variable number of point2d values into a path: "(1,2) -> (3,4)".
fn point_path(args: &[InValue]) -> VdfReturn {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(" -> ");
        }
        match arg {
            InValue::Custom(b) if b.len() >= POINT_BYTES => {
                let (x, y) = point_from_bytes(b);
                write!(out, "({x},{y})").unwrap();
            }
            InValue::Null => return VdfReturn::Null,
            _ => return VdfReturn::error("point_path: expected point2d bytes"),
        }
    }
    VdfReturn::string(out)
}

villagesql::extension! {
    funcs: [
        villagesql::varargs_func!(str_join, "str_join" -> villagesql::Type::String,
            state: JoinState, prerun: str_join_prerun),
        villagesql::varargs_func!(arg_count, "arg_count" -> villagesql::Type::Int),
        villagesql::varargs_func!(describe, "describe" -> villagesql::Type::String,
            prerun: describe_prerun),
        villagesql::varargs_func!(point_path, "point_path" -> villagesql::Type::String,
            prerun: point_path_prerun),
    ],
    types: [
        villagesql::custom_type!(
            type_name: "point2d",
            persisted_length: 8,
            max_decode_buffer_length: 32,
            encode: point_encode,
            decode: point_decode,
            compare: point_compare,
            default: "0,0",
        ),
    ]
}
