// Example crate: the parse/resolve author API takes `Params` by value by design
// (the SDK hands it over owned), and these demo helpers don't need `#[must_use]`.
// Relax the matching pedantic lints here rather than distort the example.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::must_use_candidate
)]

use villagesql::{
    BindArg, BindArgs, BindResult, InValue, MaybeParams, Params, Resolved, TypeParameters,
    VdfReturn,
};

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

pub fn counter_default() -> Result<String, String> {
    Ok("0".to_string())
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

// ── Type 3: padint (i64 in 8 bytes; `width` param sizes the decode buffer) ────

pub fn padint_encode(s: &str, params: &mut MaybeParams<PadintParams>) -> Result<Vec<u8>, String> {
    let n: i64 = s.trim().parse().map_err(|e| format!("padint: {e}"))?;
    if !params.is_known() {
        // Bare constant: infer width from the digit count of the literal.
        let digits = s.trim().trim_start_matches('-').len().max(1);
        params.set(PadintParams {
            width: u32::try_from(digits).unwrap_or(u32::MAX),
        });
    }
    Ok(n.to_le_bytes().to_vec())
}

/// Read a padint's i64 from its stored bytes, or `None` if the buffer is
/// shorter than 8 bytes. Returning `None` (instead of slicing `b[..8]`) avoids
/// a panic inside the FFI trampoline on a short/corrupt buffer.
fn read_i64(b: &[u8]) -> Option<i64> {
    b.get(..8)
        .and_then(|s| s.try_into().ok())
        .map(i64::from_le_bytes)
}

pub fn padint_decode(b: &[u8], p: &PadintParams) -> Result<String, String> {
    let n = read_i64(b).ok_or_else(|| format!("padint: expected 8 bytes, got {}", b.len()))?;
    let width = p.width as usize;
    Ok(format!("{n:0width$}"))
}

#[must_use]
pub fn padint_compare(a: &[u8], b: &[u8], _p: &PadintParams) -> std::cmp::Ordering {
    let va = read_i64(a).unwrap_or(0);
    let vb = read_i64(b).unwrap_or(0);
    va.cmp(&vb)
}

pub fn padint_int_to_params(n: i64) -> Result<String, String> {
    if n < 1 {
        return Err(format!("padint: width must be >= 1, got {n}"));
    }
    Ok(format!("width={n}"))
}

pub fn padint_resolve_params(params: Params) -> Result<Resolved, String> {
    let width: i64 = params
        .get("width")
        .ok_or_else(|| "padint: missing 'width' param".to_string())?
        .parse()
        .map_err(|e| format!("padint: bad width: {e}"))?;
    // Storage is a fixed 8-byte i64. The decode buffer must fit the widest
    // rendering: an i64 is up to 20 characters, and zero-padding can widen it
    // to `width` characters — so take the larger of the two.
    Ok(Resolved::new(8, width.max(20)))
}

#[must_use]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
pub fn padint_hash(b: &[u8], _p: &PadintParams) -> usize {
    read_i64(b).unwrap_or(0) as usize
}

pub struct PadintParams {
    pub width: u32,
}

impl TypeParameters for PadintParams {
    fn parse(params: Params) -> Self {
        let width = params
            .get("width")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        PadintParams { width }
    }

    fn to_strings(&self) -> Vec<(String, String)> {
        vec![("width".to_string(), self.width.to_string())]
    }
}

pub struct TvectorParams {
    pub dimension: i64,
    pub bytes_per_elem: usize,
}

impl TypeParameters for TvectorParams {
    fn parse(params: Params) -> Self {
        let dimension = params
            .get("dimension")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let bytes_per_elem = if params.get("type") == Some("double") {
            8
        } else {
            4
        };
        TvectorParams {
            dimension,
            bytes_per_elem,
        }
    }

    fn to_strings(&self) -> Vec<(String, String)> {
        let ty = if self.bytes_per_elem == 8 {
            "double"
        } else {
            "float"
        };
        vec![
            ("dimension".to_string(), self.dimension.to_string()),
            ("type".to_string(), ty.to_string()),
        ]
    }
}

pub fn tvector_int_to_params(n: i64) -> Result<String, String> {
    if !(1..=4096).contains(&n) {
        return Err(format!(
            "tvector: dimension must be in the range [1, 4096], got {n}"
        ));
    }
    Ok(format!("dimension={n}"))
}

pub fn tvector_resolve_params(params: Params) -> Result<Resolved, String> {
    let dimension: i64 = params
        .get("dimension")
        .ok_or_else(|| "tvector: missing 'dimension' param".to_string())?
        .parse()
        .map_err(|e| format!("tvector: bad dimension: {e}"))?;
    if !(1..=4096).contains(&dimension) {
        return Err(format!(
            "tvector: dimension must be in the range [1, 4096], got {dimension}"
        ));
    }

    // `type` is optional. Default it to "float". This is the mutating form:
    // we rewrite the params so the resolved `type` is persisted explicitly.
    let ty = params.get("type").unwrap_or("float");
    let bytes_per_elem: i64 = match ty {
        "float" => 4,
        "double" => 8,
        other => {
            return Err(format!(
                "tvector: type must be 'float' or 'double', got {other:?}"
            ))
        }
    };

    let persisted_length = dimension * bytes_per_elem;
    let max_decode = dimension * if bytes_per_elem == 8 { 32 } else { 16 };

    Ok(Resolved::rewrite(
        persisted_length,
        max_decode,
        vec![
            ("dimension".to_string(), dimension.to_string()),
            ("type".to_string(), ty.to_string()),
        ],
    ))
}

/// Parse a tvector's text form, `[a,b,c]`, into its elements.
fn parse_tvector_text(s: &str) -> Result<Vec<f64>, String> {
    let inner = s
        .trim()
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| "tvector: expected '[...]'".to_string())?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|t| {
            t.trim()
                .parse::<f64>()
                .map_err(|e| format!("tvector: bad element: {e}"))
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
pub fn tvector_encode(s: &str, params: &mut MaybeParams<TvectorParams>) -> Result<Vec<u8>, String> {
    let elems = parse_tvector_text(s)?;

    // Params from the column, or inferred from the literal (dimension = count).
    let (dimension, bytes_per_elem) = if let Some(p) = params.get() {
        (p.dimension, p.bytes_per_elem)
    } else {
        let dim = i64::try_from(elems.len()).unwrap_or(0);
        params.set(TvectorParams {
            dimension: dim,
            bytes_per_elem: 4,
        });
        (dim, 4)
    };

    if i64::try_from(elems.len()).unwrap_or(-1) != dimension {
        return Err(format!(
            "tvector: expected {dimension} elements, got {}",
            elems.len()
        ));
    }

    let mut bytes = Vec::with_capacity(elems.len() * bytes_per_elem);
    for e in &elems {
        if bytes_per_elem == 8 {
            bytes.extend_from_slice(&e.to_le_bytes());
        } else {
            bytes.extend_from_slice(&(*e as f32).to_le_bytes());
        }
    }
    Ok(bytes)
}

/// Read the element (as f64) in `bytes` starting at `start`, or `None` if the
/// bytes are too short. Returning `None` (instead of slicing) avoids a panic in
/// the FFI trampoline on a short/corrupt buffer. Shared by decode and compare.
fn read_elem(bytes: &[u8], start: usize, elem_size: usize) -> Option<f64> {
    let chunk = bytes.get(start..start + elem_size)?;
    Some(if elem_size == 8 {
        f64::from_le_bytes(chunk.try_into().ok()?)
    } else {
        f64::from(f32::from_le_bytes(chunk.try_into().ok()?))
    })
}

pub fn tvector_decode(bytes: &[u8], params: &TvectorParams) -> Result<String, String> {
    let dim = usize::try_from(params.dimension).unwrap_or(0);
    let elem_size = params.bytes_per_elem; // bytes per number (4 = float, 8 = double)
    let mut out = String::from("[");
    for i in 0..dim {
        if i > 0 {
            out.push(',');
        }
        let val = read_elem(bytes, i * elem_size, elem_size)
            .ok_or_else(|| "tvector: buffer too small".to_string())?;
        out.push_str(&val.to_string());
    }
    out.push(']');
    Ok(out)
}

#[must_use]
pub fn tvector_compare(a: &[u8], b: &[u8], params: &TvectorParams) -> std::cmp::Ordering {
    let dim = usize::try_from(params.dimension).unwrap_or(0);
    let elem_size = params.bytes_per_elem; // bytes per number (4 = float, 8 = double)
    for i in 0..dim {
        let start = i * elem_size;
        let val_a = read_elem(a, start, elem_size).unwrap_or(0.0);
        let val_b = read_elem(b, start, elem_size).unwrap_or(0.0);
        match val_a.partial_cmp(&val_b) {
            Some(std::cmp::Ordering::Equal) | None => {}
            Some(ord) => return ord,
        }
    }
    std::cmp::Ordering::Equal
}

pub fn tvector_default(p: &TvectorParams) -> Result<String, String> {
    let n = usize::try_from(p.dimension).unwrap_or(0);
    Ok(format!("[{}]", vec!["0"; n].join(",")))
}

// ── bind_and_check_types hooks ────────────────────────────────────────────────
//
// tvector_concat and tvector_concat_all port the C++ SDK's vsql-tvector
// example. padint_with_width is the TYPEID('user') pattern: the result's type
// parameters come from a constant string argument.

const TVECTOR_MAX_DIMENSION: i64 = 4096;
/// Largest tvector in bytes: the maximum dimension of 8-byte doubles.
const TVECTOR_MAX_BYTES: usize = 32_768;
/// A literal has no element type of its own; float is tvector's default.
const DEFAULT_BYTES_PER_ELEM: usize = 4;

/// The element width every argument will use. A constant has none of its own,
/// so it takes whichever argument the server resolved; where more than one is
/// resolved they must agree.
fn common_element_width(args: BindArgs) -> Result<usize, String> {
    let mut resolved: Option<usize> = None;
    for i in 0..args.len() {
        let Some(p) = args.at(i).and_then(|a| a.params::<TvectorParams>()) else {
            continue;
        };
        if resolved.is_some_and(|bpe| bpe != p.bytes_per_elem) {
            return Err("vectors must have the same element type".to_string());
        }
        resolved = Some(p.bytes_per_elem);
    }
    Ok(resolved.unwrap_or(DEFAULT_BYTES_PER_ELEM))
}

/// One argument's tvector params: the server's, when it resolved them, or
/// counted from a constant literal. The flag is true for the latter, meaning
/// the caller must hand the params back with `set_arg` so the server can
/// encode the literal.
fn bind_params_for_vector(
    arg: &BindArg,
    index: usize,
    bpe: usize,
) -> Result<(TvectorParams, bool), String> {
    if let Some(known) = arg.params::<TvectorParams>() {
        return Ok((known, false));
    }
    let n = index + 1;
    let text = arg
        .const_str()
        .ok_or_else(|| format!("argument {n} must be a tvector or a constant vector literal"))?;
    let elems =
        parse_tvector_text(text).map_err(|_| format!("argument {n} is not a vector literal"))?;
    if elems.is_empty() {
        return Err(format!(
            "argument {n} is empty; a tvector must have at least one element"
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    if bpe == 4 && elems.iter().any(|e| !(*e as f32).is_finite()) {
        return Err(format!(
            "argument {n} has an element that is not a finite float value"
        ));
    }
    let dimension = i64::try_from(elems.len()).unwrap_or(i64::MAX);
    if dimension > TVECTOR_MAX_DIMENSION {
        return Err(format!(
            "argument {n} has {dimension} elements; the maximum is {TVECTOR_MAX_DIMENSION}"
        ));
    }
    Ok((
        TvectorParams {
            dimension,
            bytes_per_elem: bpe,
        },
        true,
    ))
}

/// The shared half of both concat hooks: resolve every argument as a tvector,
/// hand back params for the literals, and return the concatenation's params.
/// A varargs call has no signature saying its arguments are tvectors, so there
/// the literal's type is named too.
fn bind_concat(
    args: BindArgs,
    out: &mut BindResult,
    varargs: bool,
) -> Result<TvectorParams, String> {
    let bpe = common_element_width(args)?;
    let mut total = 0;
    for i in 0..args.len() {
        let Some(arg) = args.at(i) else { continue };
        let (params, needs_publish) = bind_params_for_vector(&arg, i, bpe)?;
        total += params.dimension;
        if needs_publish {
            if varargs {
                out.set_arg_with_type(i, "tvector", &params);
            } else {
                out.set_arg(i, &params);
            }
        }
    }
    if total > TVECTOR_MAX_DIMENSION {
        return Err(format!(
            "combined dimension {total} exceeds the maximum of {TVECTOR_MAX_DIMENSION}"
        ));
    }
    Ok(TvectorParams {
        dimension: total,
        bytes_per_elem: bpe,
    })
}

/// `tvector_concat(tvector(M), tvector(N))` -> `tvector(M+N)`. The built-in
/// rules would reject arguments whose dimensions differ, and could only copy M
/// or N onto the result.
fn tvector_concat_bind(args: BindArgs, mut out: BindResult) {
    if args.len() != 2 {
        out.error("tvector_concat: expects 2 arguments");
        return;
    }
    match bind_concat(args, &mut out, false) {
        Ok(ret) => out.set_return(&ret),
        Err(e) => out.error(&format!("tvector_concat: {e}")),
    }
}

/// The variadic concat. Every argument must be a tvector or a constant literal.
fn tvector_concat_all_bind(args: BindArgs, mut out: BindResult) {
    if args.is_empty() {
        out.error("tvector_concat_all: requires at least one argument");
        return;
    }
    for i in 0..args.len() {
        let name = args
            .at(i)
            .and_then(|a| a.ty().custom_name().map(str::to_string));
        if let Some(name) = name.filter(|n| !n.eq_ignore_ascii_case("tvector")) {
            out.error(&format!(
                "tvector_concat_all: argument {} is a {name}, not a tvector",
                i + 1
            ));
            return;
        }
    }
    match bind_concat(args, &mut out, true) {
        Ok(ret) => out.set_return(&ret),
        Err(e) => out.error(&format!("tvector_concat_all: {e}")),
    }
}

/// Row function for both concats. The hook made every argument the same
/// element width, so concatenating the stored bytes is the whole job.
fn tvector_concat_impl(args: &[InValue]) -> VdfReturn {
    let mut bytes = Vec::new();
    for arg in args {
        match arg {
            InValue::CustomWithParams { bytes: b, .. } | InValue::Custom(b) => {
                bytes.extend_from_slice(b);
            }
            InValue::Null => return VdfReturn::null(),
            _ => return VdfReturn::error("tvector_concat: expected tvector arguments"),
        }
    }
    VdfReturn::binary(bytes)
}

/// `padint_with_width('5', 42)` -> `padint(5)`: the result's width comes from
/// the constant string, the way `TYPEID('user')` would take its prefix.
fn padint_with_width_bind(args: BindArgs, mut out: BindResult) {
    let Some(text) = args.at(0).and_then(|a| a.const_str()) else {
        out.error("padint_with_width: the width must be a constant string");
        return;
    };
    match text.trim().parse::<u32>() {
        Ok(width) if width >= 1 => out.set_return(&PadintParams { width }),
        _ => out.error(&format!(
            "padint_with_width: width must be a positive integer, got {text:?}"
        )),
    }
}

fn padint_with_width_impl(args: &[InValue]) -> VdfReturn {
    match args.get(1) {
        Some(InValue::Int(n)) => VdfReturn::binary(n.to_le_bytes().to_vec()),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("padint_with_width: expected an INT value"),
    }
}

// ── Extension registration ────────────────────────────────────────────────────

villagesql::extension! {
    funcs: [
        villagesql::func!(sdk_identity_int_impl, "sdk_identity_int",
            [villagesql::Type::Int] -> villagesql::Type::Int),
        villagesql::func!(sdk_identity_real_impl, "sdk_identity_real",
            [villagesql::Type::Real] -> villagesql::Type::Real),
        villagesql::func!(sdk_warn_if_negative_impl, "sdk_warn_if_negative",
            [villagesql::Type::Int] -> villagesql::Type::Int),
        villagesql::func!(tvector_concat_impl, "tvector_concat",
            [villagesql::custom!("tvector"), villagesql::custom!("tvector")]
                -> villagesql::custom!("tvector"),
            buffer_size: TVECTOR_MAX_BYTES, deterministic: true, bind: tvector_concat_bind),
        villagesql::varargs_func!(tvector_concat_impl, "tvector_concat_all",
            [..] -> villagesql::custom!("tvector"),
            buffer_size: TVECTOR_MAX_BYTES, deterministic: true, bind: tvector_concat_all_bind),
        villagesql::func!(padint_with_width_impl, "padint_with_width",
            [villagesql::Type::String, villagesql::Type::Int] -> villagesql::custom!("padint"),
            deterministic: true, bind: padint_with_width_bind),
    ],
    types: [
        villagesql::custom_type!(
            type_name: "counter",
            persisted_length: 8,
            max_decode_buffer_length: 20,
            encode: counter_encode,
            decode: counter_decode,
            compare: counter_compare,
            hash: counter_hash,
            intrinsic_default_fn: counter_default,
        ),
        villagesql::custom_type!(
            type_name: "flag",
            persisted_length: 1,
            max_decode_buffer_length: 5,
            encode: flag_encode,
            decode: flag_decode,
            compare: flag_compare,
            default: "false",
        ),
        villagesql::parameterized_type!(
            type_name: "padint",
            max_persisted_length: 8,
            max_decode_buffer_length: 32,
            encode: padint_encode,
            decode: padint_decode,
            compare: padint_compare,
            int_to_params: padint_int_to_params,
            resolve_params: padint_resolve_params,
            params_type: PadintParams,
            hash: padint_hash,
            default: "0",
        ),
        villagesql::parameterized_type!(
            type_name: "tvector",
            max_persisted_length: 32768,
            max_decode_buffer_length: 131_072,
            encode: tvector_encode,
            decode: tvector_decode,
            compare: tvector_compare,
            int_to_params: tvector_int_to_params,
            resolve_params: tvector_resolve_params,
            params_type: TvectorParams,
            intrinsic_default_fn: tvector_default,
        ),
    ]
}
