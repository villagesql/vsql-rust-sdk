//! Safe Rust SDK for writing `VillageSQL` extension functions (VDFs) and custom types.
//!
//! # Quick start (functions)
//!
//! 1. Add `villagesql = "0.0.1"` to your `Cargo.toml` with `crate-type = ["cdylib"]`.
//! 2. Write a function with the signature `fn(&[InValue]) -> VdfReturn`.
//! 3. Declare the extension with the [`extension!`] macro.
//!
//! See `examples/vsql_rot13` for a complete working function extension.
//! See `examples/vsql_rational` for a complete working custom type extension.

pub use paste;
pub use villagesql_sys as sys;
pub mod preview;

use core::ffi::c_void;
use std::collections::HashMap;
use std::ffi::c_char;
use std::marker::PhantomData;
use std::sync::{OnceLock, RwLock};
use villagesql_sys::{
    vef_func_desc_t, vef_postrun_args_t, vef_prerun_args_t, vef_prerun_result_t,
    vef_protocol_t_VEF_PROTOCOL_3, vef_registration_t, vef_required_capability_t,
    vef_return_value_type_t_VEF_RESULT_ERROR, vef_return_value_type_t_VEF_RESULT_NULL,
    vef_return_value_type_t_VEF_RESULT_VALUE, vef_return_value_type_t_VEF_RESULT_WARNING,
    vef_signature_t, vef_type_desc_t, vef_type_id_VEF_TYPE_CUSTOM, vef_type_id_VEF_TYPE_INT,
    vef_type_id_VEF_TYPE_REAL, vef_type_id_VEF_TYPE_STRING, vef_type_params_t, vef_type_t,
    vef_vdf_args_t, vef_vdf_result_t, vef_version_t, VEF_MAX_ERROR_LEN,
};

use crate::preview::RequiredCapability;

// ── Public types ──────────────────────────────────────────────────────────────

/// SQL type identifier for a VDF parameter or return type.
///
/// Use the [`custom!`] macro to construct the `Custom` variant rather than
/// building it directly, as it must hold a null-terminated static C string.
#[derive(Copy, Clone, Debug)]
pub enum Type {
    String,
    Real,
    Int,
    /// A custom type registered by this extension. The pointer must be a
    /// null-terminated UTF-8 string with `'static` lifetime. Use [`custom!`].
    Custom(*const c_char),
}

// SAFETY: all variants hold only static data or fn pointers.
unsafe impl Send for Type {}
unsafe impl Sync for Type {}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Type::String, Type::String) | (Type::Real, Type::Real) | (Type::Int, Type::Int) => {
                true
            }
            (Type::Custom(a), Type::Custom(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Type {}

impl Type {
    fn to_raw(self) -> vef_type_t {
        match self {
            Type::String => vef_type_t {
                id: vef_type_id_VEF_TYPE_STRING,
                custom_type: std::ptr::null(),
            },
            Type::Real => vef_type_t {
                id: vef_type_id_VEF_TYPE_REAL,
                custom_type: std::ptr::null(),
            },
            Type::Int => vef_type_t {
                id: vef_type_id_VEF_TYPE_INT,
                custom_type: std::ptr::null(),
            },
            Type::Custom(name) => vef_type_t {
                id: vef_type_id_VEF_TYPE_CUSTOM,
                custom_type: name,
            },
        }
    }
}

/// A single input value delivered to a VDF for one row.
///
/// Always check for [`InValue::Null`] before attempting to read the inner value.
/// For custom types the binary persisted bytes are exposed as
/// [`InValue::Custom`]. If it's a custom type with parameters, then the binary
/// persisted bytes and parameters are exposed as [`InValue::CustomWithParams`].
#[derive(Debug)]
pub enum InValue<'a> {
    Null,
    String(&'a str),
    Real(f64),
    Int(i64),
    /// A custom-type argument: just its raw persisted bytes.
    Custom(&'a [u8]),
    /// A custom-type argument: its raw persisted bytes plus the type parameters
    /// the column was declared with.
    CustomWithParams {
        bytes: &'a [u8],
        params: TypeParams<'a>,
    },
}

/// Read only view of a parameterized custom type's parameters.
///
/// Parameters arrive as canonical `key=value` pairs (e.g. `dimension=536`).
/// This is a borrowed, zero-copy view over the server provided arrays: a pair
/// is only decoded when you read it, so there is no per row allocation.
#[derive(Copy, Clone)]
pub struct TypeParams<'a> {
    raw: &'a vef_type_params_t,
}

impl<'a> TypeParams<'a> {
    /// How many parameters there are. Zero for non-parameterized types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.count as usize
    }

    /// True when there are no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up one parameter by name, e.g. `params.get("max_size")`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// Walk through every `(key, value)` pair.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        let (keys, values) = (self.raw.keys, self.raw.values);
        (0..self.len()).map(move |i| unsafe {
            // SAFETY: for every index below `count`, the server guarantees
            // keys[i]/values[i] point to valid, NUL-terminated UTF-8 strings.
            (cstr_to_str(*keys.add(i)), cstr_to_str(*values.add(i)))
        })
    }
}

impl std::fmt::Debug for TypeParams<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Parsed, borrowed view of a custom type's canonical parameter string
/// (`key=value,key=value,...`). Built by the SDK from the `resolve_params`
/// argument; borrows the input, so there are no string copies.
#[derive(Debug)]
pub struct Params<'a> {
    pairs: Vec<(&'a str, &'a str)>,
}

impl<'a> Params<'a> {
    /// Parse a canonical params string. An empty string yields no params.
    #[must_use]
    pub fn parse(s: &'a str) -> Self {
        let mut pairs = Vec::new();
        if !s.is_empty() {
            for entry in s.split(',') {
                // Match the C++ SDK: an entry with no '=' becomes a key
                // with an empty value, rather than being dropped.
                match entry.split_once('=') {
                    Some((k, v)) => pairs.push((k, v)),
                    None => pairs.push((entry, "")),
                }
            }
        }
        Self { pairs }
    }

    /// Look up one parameter value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    /// Walk every `(key, value)` pair, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        self.pairs.iter().copied()
    }

    /// Number of parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// True when there are no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Type parameters that may or may not be known yet.
///
/// On the normal (row-time) path a custom value's params are already known.
/// On the inference path (a bare constant with no column to anchor it) they
/// start unknown, and the type's `encode` fn infers them from the value and
/// calls [`MaybeParams::set`]. The framework then publishes them back to the
/// server.
pub struct MaybeParams<P> {
    params: Option<P>,
}

impl<P> MaybeParams<P> {
    /// Construct in the unknown state (params to be inferred).
    #[must_use]
    pub fn empty() -> Self {
        Self { params: None }
    }

    /// Construct in the known state.
    #[must_use]
    pub fn present(params: P) -> Self {
        Self {
            params: Some(params),
        }
    }

    /// True if the params are already known.
    #[must_use]
    pub fn is_known(&self) -> bool {
        self.params.is_some()
    }

    /// The params, if known.
    #[must_use]
    pub fn get(&self) -> Option<&P> {
        self.params.as_ref()
    }

    /// Record the inferred params (transitions unknown to known).
    pub fn set(&mut self, params: P) {
        self.params = Some(params);
    }
}

/// The outcome of a parameterized type's `resolve_params`.
///
/// Carries the resolved storage sizes, and optionally a rewritten parameter
/// set (the mutating form) used to fill in defaults. When present, the server
/// persists the rewritten params in place of the input.
pub struct Resolved {
    persisted_length: i64,
    max_decode_buffer_length: i64,
    rewritten: Option<Vec<(String, String)>>,
}

impl Resolved {
    /// The const form: just the resolved sizes. Params left unchanged.
    #[must_use]
    pub fn new(persisted_length: i64, max_decode_buffer_length: i64) -> Self {
        Self {
            persisted_length,
            max_decode_buffer_length,
            rewritten: None,
        }
    }

    /// The mutating form: resolved sizes plus a rewritten param set that
    /// replaces the input (e.g. to fill defaults).
    #[must_use]
    pub fn rewrite(
        persisted_length: i64,
        max_decode_buffer_length: i64,
        params: Vec<(String, String)>,
    ) -> Self {
        Self {
            persisted_length,
            max_decode_buffer_length,
            rewritten: Some(params),
        }
    }

    /// Serialize to the string form the server expects from `resolve_params`:
    /// - const:    `<persisted_length>,<max_decode_buffer_length>`
    /// - mutating: `<p>,<m>,<byte-len>[,<canonical params>]`
    ///
    /// The byte length makes the training params section self-delimiting.
    #[must_use]
    pub fn to_wire_string(&self) -> String {
        let base = format!(
            "{},{}",
            self.persisted_length, self.max_decode_buffer_length
        );
        let Some(pairs) = &self.rewritten else {
            return base; // returns here with const form
        };

        let canonical =
            canonical_params_string(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));

        let out = if canonical.is_empty() {
            format!("{base},0")
        } else {
            format!("{base},{},{canonical}", canonical.len())
        };
        out
    }
}

/// A thread-safe, parse-once cache mapping a canonical params string to its
/// typed form `P`. One concrete cache exists per parameterized type. The
/// `parameterized_type!` macro creates a `static` holding it.
pub struct TypeParamsCache<P: 'static> {
    map: OnceLock<RwLock<HashMap<String, &'static P>>>,
}

impl<P: Send + Sync + 'static> TypeParamsCache<P> {
    /// Create an empty cache. `const` so it can initialize a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            map: OnceLock::new(),
        }
    }

    fn map(&self) -> &RwLock<HashMap<String, &'static P>> {
        self.map.get_or_init(|| RwLock::new(HashMap::new()))
    }

    /// Typed params for `raw`, parsing + caching on first sight.
    ///
    /// # Panics
    /// Panics if the cache lock is poisoned (a thread panicked while holding it).
    pub fn get(&self, raw: TypeParams, parse: fn(Params) -> P) -> &'static P {
        let key = canonical_key(raw);

        // Fast path: shared read lock; most calls hit an existing entry.
        if let Some(&p) = self.map().read().unwrap().get(&key) {
            return p;
        }

        // Miss: exclusive write lock, re-check (another thread may have been
        // in a race), then parse once and insert.
        let mut map = self.map().write().unwrap();
        if let Some(&p) = map.get(&key) {
            return p;
        }
        let parsed: &'static P = Box::leak(Box::new(parse(Params::parse(&key))));
        map.insert(key, parsed);
        parsed
    }
}

impl<P: Send + Sync + 'static> Default for TypeParamsCache<P> {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the canonical `key=value,key=value` string with keys sorted — the
/// exact wire form the server expects. Single source of truth for the three
/// places that need it (cache key, `resolve_params` output, inferred params).
fn canonical_params_string<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut pairs: Vec<(&str, &str)> = pairs.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

fn canonical_key(raw: TypeParams) -> String {
    canonical_params_string(raw.iter())
}

/// The value a VDF returns for one row.
#[derive(Debug)]
pub enum VdfReturn {
    /// SQL NULL result.
    Null,
    /// A string value.
    String(std::string::String),
    /// A floating-point value.
    Real(f64),
    /// An integer value.
    Int(i64),
    /// Raw binary bytes for a custom-type return value (persisted format).
    Binary(Vec<u8>),
    /// Row-level warning: execution continues, NULL is returned for this row.
    Warning(std::string::String),
    /// Fatal error: statement execution is aborted.
    Error(std::string::String),
}

impl VdfReturn {
    #[must_use]
    pub fn null() -> Self {
        Self::Null
    }
    pub fn string(s: impl Into<std::string::String>) -> Self {
        Self::String(s.into())
    }
    #[must_use]
    pub fn real(v: f64) -> Self {
        Self::Real(v)
    }
    #[must_use]
    pub fn int(v: i64) -> Self {
        Self::Int(v)
    }
    #[must_use]
    pub fn binary(v: Vec<u8>) -> Self {
        Self::Binary(v)
    }
    pub fn warning(msg: impl Into<std::string::String>) -> Self {
        Self::Warning(msg.into())
    }
    pub fn error(msg: impl Into<std::string::String>) -> Self {
        Self::Error(msg.into())
    }
}

// ── Function descriptor ───────────────────────────────────────────────────────

/// Compile-time descriptor for a single VDF. Built by [`func!`]; do not
/// construct this directly.
pub struct FuncDescriptor {
    pub sql_name: *const c_char,
    pub params: &'static [Type],
    pub returns: Type,
    pub trampoline: unsafe extern "C" fn(
        *mut villagesql_sys::vef_context_t,
        *mut vef_vdf_args_t,
        *mut vef_vdf_result_t,
    ),
    pub buffer_size: usize,
    pub deterministic: bool,
    pub varargs: bool,
    pub prerun: villagesql_sys::vef_prerun_func_t,
    pub postrun: villagesql_sys::vef_postrun_func_t,
    pub clear: villagesql_sys::vef_vdf_clear_func_t,
    pub accumulate: villagesql_sys::vef_vdf_accumulate_func_t,
}

unsafe impl Send for FuncDescriptor {}
unsafe impl Sync for FuncDescriptor {}

// ── Type descriptor ───────────────────────────────────────────────────────────

/// Compile-time descriptor for a custom SQL type. Built by [`custom_type!`];
/// do not construct this directly.
pub struct TypeDescriptor {
    pub sql_name: *const c_char,
    /// Fixed binary size in bytes for persisted storage.
    pub persisted_length: i64,
    /// Upper bound on the string representation length (for decode output).
    pub max_decode_buffer_length: i64,
    /// Null-terminated name of the `TYPE::from_string` VDF.
    pub encode_vdf_name: *const c_char,
    /// Null-terminated name of the `TYPE::to_string` VDF.
    pub decode_vdf_name: *const c_char,
    /// Null-terminated name of the `TYPE::compare` VDF.
    pub compare_vdf_name: *const c_char,
    /// Null-terminated name of the `TYPE::hash` VDF, or null if not provided.
    pub hash_vdf_name: *const c_char,
    /// Null-terminated name of the `TYPE::int_to_params` VDF, or null if not parameterized.
    pub int_to_params_vdf_name: *const c_char,
    /// Null-terminated name of the `TYPE::resolve_params` VDF, or null if not parameterized.
    pub resolve_params_vdf_name: *const c_char,
    /// Upper bound on persisted bytes across all parameter values. 0 for non-parameterized types.
    pub max_persisted_length: i64,
    /// Optional null-terminated default value string (encoded at install time).
    pub intrinsic_default_str: *const c_char,
    /// Null-terminated name of the `TYPE::intrinsic_default` VDF, or null.
    pub intrinsic_default_vdf_name: *const c_char,
}

unsafe impl Send for TypeDescriptor {}
unsafe impl Sync for TypeDescriptor {}

/// A custom type together with its embedded SQL-callable VDFs
/// (`TYPE::from_string`, `TYPE::to_string`, `TYPE::compare`, `TYPE::hash`).
/// Built by [`custom_type!`]; do not construct this directly.
pub struct TypeWithFuncs {
    pub descriptor: TypeDescriptor,
    pub embedded_funcs: Vec<FuncDescriptor>,
}

unsafe impl Send for TypeWithFuncs {}
unsafe impl Sync for TypeWithFuncs {}

// Prerun / Postrun
#[derive(Clone, Copy)]
pub struct PrerunArgs<'a> {
    inner: &'a villagesql_sys::vef_prerun_args_t,
}

impl<'a> PrerunArgs<'a> {
    pub(crate) unsafe fn from_raw(raw: *const villagesql_sys::vef_prerun_args_t) -> Self {
        Self { inner: &*raw }
    }

    /// Number of arguments each row will receive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.arg_count as usize
    }

    /// True if the function was called with no arguments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The type of argument `i`, or `None` if `i` is out of range.
    #[must_use]
    pub fn type_at(&self, i: usize) -> Option<ArgType<'a>> {
        if i >= self.len() {
            return None;
        }
        // SAFETY: the server guarantees `arg_types` points to `arg_count` valid entries.
        let types = unsafe { std::slice::from_raw_parts(self.inner.arg_types, self.len()) };
        Some(ArgType { inner: &types[i] })
    }
}

/// The type of one argument, as seen at prerun time. A borrowed, read-only view.
pub struct ArgType<'a> {
    inner: &'a villagesql_sys::vef_type_t,
}

impl ArgType<'_> {
    #[must_use]
    pub fn is_str(&self) -> bool {
        self.inner.id == villagesql_sys::vef_type_id_VEF_TYPE_STRING
    }
    #[must_use]
    pub fn is_real(&self) -> bool {
        self.inner.id == villagesql_sys::vef_type_id_VEF_TYPE_REAL
    }
    #[must_use]
    pub fn is_int(&self) -> bool {
        self.inner.id == villagesql_sys::vef_type_id_VEF_TYPE_INT
    }
    #[must_use]
    pub fn is_custom(&self) -> bool {
        self.inner.id == villagesql_sys::vef_type_id_VEF_TYPE_CUSTOM
    }

    /// The custom type's name, or `None` if this isn't a custom type.
    #[must_use]
    pub fn custom_name(&self) -> Option<&str> {
        if self.is_custom() && !self.inner.custom_type.is_null() {
            // SAFETY: server guarantees a valid NUL-terminated UTF-8 name for custom types.
            Some(unsafe { cstr_to_str(self.inner.custom_type) })
        } else {
            None
        }
    }
}

/// Handle a prerun uses to set up a statement: `set_state`, `request_buffer_size`,
/// or `error`. `T` is the state type, matched to the `&mut T` the row handler
/// gets. The compiler ensures the types match up.
pub struct PrerunResult<'a, T> {
    inner: &'a mut villagesql_sys::vef_prerun_result_t,
    _marker: PhantomData<fn() -> T>,
}

impl<T> PrerunResult<'_, T> {
    pub(crate) unsafe fn from_raw(raw: *mut villagesql_sys::vef_prerun_result_t) -> Self {
        Self {
            inner: &mut *raw,
            _marker: PhantomData,
        }
    }

    /// Allocate the per-statement state and hand it to the server.
    /// Consumes `self` instead of borrowing so you can't accidentally call it twice.
    pub fn set_state(self, state: T) {
        self.inner.user_data = Box::into_raw(Box::new(state)).cast::<c_void>();
    }

    /// Ask the server for a specific result-buffer size.
    pub fn request_buffer_size(&mut self, n: usize) {
        self.inner.result_buffer_size = n;
    }

    /// Fail the whole statement with a message.
    pub fn error(self, msg: &str) {
        self.inner.type_ = vef_return_value_type_t_VEF_RESULT_ERROR;
        unsafe {
            write_error_msg(msg.as_bytes(), self.inner.error_msg);
        }
    }
}

// ── Internal runtime helpers ──────────────────────────────────────────────────

/// Borrow the state stashed by `set_state`, without taking ownership.
///
/// # Safety
/// `raw` must be the pointer produced by `set_state` for this statement,
/// still alive (postrun hasn't run yet), with no other live borrow of it.
unsafe fn borrow_state<'a, T>(raw: *mut c_void) -> &'a mut T {
    debug_assert!(!raw.is_null(), "state pointer was null in row handler");
    &mut *(raw.cast::<T>())
}

/// Reclaim and drop the boxed state, freeing its memory.
///
/// # Safety
/// `raw` must be a pointer from `set_state` that hasn't been reclaimed yet.
/// Call exactly once, from postrun.
unsafe fn reclaim_state<T>(raw: *mut c_void) {
    if raw.is_null() {
        return;
    }
    drop(Box::from_raw(raw.cast::<T>()));
}

/// Helper to convert raw Protocol-3 VDF arguments into a `&[InValue]` slice.
///
/// # Safety
/// `args` and `result` must be valid for the duration of the call.
unsafe fn read_in_values(args: &vef_vdf_args_t) -> Vec<InValue<'_>> {
    let value_count = args.value_count as usize;
    let raw_vals = std::slice::from_raw_parts(args.__bindgen_anon_1.values, value_count);

    let mut in_values: Vec<InValue> = Vec::with_capacity(value_count);
    for &ptr in raw_vals {
        let v = &*ptr;
        if v.is_null {
            in_values.push(InValue::Null);
            continue;
        }
        let iv = match v.type_ {
            t if t == vef_type_id_VEF_TYPE_STRING => {
                let anon = &v.__bindgen_anon_1.__bindgen_anon_1;
                let bytes = std::slice::from_raw_parts(anon.str_value.cast::<u8>(), anon.str_len);
                InValue::String(std::str::from_utf8_unchecked(bytes))
            }
            t if t == vef_type_id_VEF_TYPE_REAL => InValue::Real(v.__bindgen_anon_1.real_value),
            t if t == vef_type_id_VEF_TYPE_INT => InValue::Int(v.__bindgen_anon_1.int_value),
            t if t == vef_type_id_VEF_TYPE_CUSTOM => {
                let anon = &v.__bindgen_anon_1.__bindgen_anon_2;
                let bytes = std::slice::from_raw_parts(anon.bin_value, anon.bin_len);
                let params = TypeParams {
                    raw: &anon.type_params,
                };
                if params.is_empty() {
                    InValue::Custom(bytes)
                } else {
                    InValue::CustomWithParams { bytes, params }
                }
            }
            _ => InValue::Null,
        };
        in_values.push(iv);
    }
    in_values
}
/// Convert raw Protocol-3 VDF arguments into a `&[InValue]` slice and call `f`.
///
/// # Safety
/// `args` and `result` must be valid for the duration of the call.
pub unsafe fn dispatch_vdf(
    f: fn(&[InValue]) -> VdfReturn,
    args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let in_values = read_in_values(&*args);
    write_result(f(&in_values), &mut *result);
}

/// Prerun runner: wrap the raw slots and call the author's setup fn.
///
/// # Safety
/// `args` and `result` must be valid for the duration of the call.
pub unsafe fn dispatch_prerun<T>(
    f: fn(PrerunArgs, PrerunResult<T>),
    args: *mut vef_prerun_args_t,
    result: *mut vef_prerun_result_t,
) {
    f(
        PrerunArgs::from_raw(args),
        PrerunResult::<T>::from_raw(result),
    );
}

/// Row runner for functions that own per-statement state. Borrows the state
/// stashed by prerun and reads the row's values, then calls the author's row fn
/// with both.
///
/// # Safety
/// `args` and `result` are valid because state was set by a prior prerun.
/// The state must have been set by a prior prerun, and `T` must watch that
/// state's type.
pub unsafe fn dispatch_vdf_with_state<T>(
    f: fn(&mut T, &[InValue]) -> VdfReturn,
    args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let args = &*args;
    let state = borrow_state::<T>(args.user_data);
    let in_values = read_in_values(args);
    write_result(f(state, &in_values), &mut *result);
}

/// Postrun runner: reclaim the state and drop the state allocated in prerun.
///
/// # Safety
/// `args` is valid (set by prerun). `T` matches what prerun set.
pub unsafe fn dispatch_postrun<T>(args: *mut vef_postrun_args_t) {
    reclaim_state::<T>((*args).user_data);
}

/// Aggregate prerun: allocate the accumulator at its default value.
///
/// # Safety
/// `result` must be valid for the duration of the call.
pub unsafe fn dispatch_agg_prerun<T: Default>(
    _args: *mut vef_prerun_args_t,
    result: *mut vef_prerun_result_t,
) {
    PrerunResult::<T>::from_raw(result).set_state(T::default());
}

/// Aggregate clear: reset the accumulator at the start of each group.
///
/// # Safety
/// `args` is valid. Its `user_data` was set by a prior prerun. `T` matches.
pub unsafe fn dispatch_clear<T>(f: fn(&mut T), args: *mut vef_vdf_args_t) {
    let state = borrow_state::<T>((*args).user_data);
    f(state);
}

/// Aggregate accumulate: fold one row into the accumulator. Returns nothing.
///
/// # Safety
/// `args` is valid. Its `user_data` was set by a prior prerun. `T` matches.
pub unsafe fn dispatch_accumulate<T>(f: fn(&mut T, &[InValue]), args: *mut vef_vdf_args_t) {
    let args = &*args;
    let state = borrow_state::<T>(args.user_data);
    let in_values = read_in_values(args);
    f(state, &in_values);
}

/// Aggregate result: produce the group's output from the finished accumulator.
/// Called once per group, after the last row is folded in.
///
/// # Safety
/// `args`/`result` are valid. `user_data` was set by a prior prerun. `T` matches.
pub unsafe fn dispatch_agg_result<T>(
    f: fn(&T) -> VdfReturn,
    args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let state = borrow_state::<T>((*args).user_data);
    write_result(f(state), &mut *result);
}

/// Dispatch a parameterized type's `from_string` (encode), including the
/// constant-string inference path.
///
/// Reads the STRING argument, builds a [`MaybeParams<P>`] from the input params
/// the server attached (present iff `count > 0`), calls the author's `encode`
/// (which may infer + `set` params), writes the encoded bytes, and on the
/// inference path publishes the inferred params back via `out_type_params`.
///
/// # Safety
/// `args` and `result` must be valid for the duration of the call.
pub unsafe fn dispatch_from_string_typed<P>(
    encode: fn(&str, &mut MaybeParams<P>) -> Result<Vec<u8>, String>,
    parse: fn(Params) -> P,
    to_strings: fn(&P) -> Vec<(String, String)>,
    args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let args = &*args;
    let result = &mut *result;

    // Pull the sole STRING argument out of the raw arg array. A NULL or missing
    // argument yields a NULL result. Anything else is a usage error.
    let raw_vals =
        std::slice::from_raw_parts(args.__bindgen_anon_1.values, args.value_count as usize);
    let s: &str = match raw_vals.first().map(|&ptr| &*ptr) {
        Some(v) if !v.is_null && v.type_ == vef_type_id_VEF_TYPE_STRING => {
            let anon = &v.__bindgen_anon_1.__bindgen_anon_1;
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                anon.str_value.cast::<u8>(),
                anon.str_len,
            ))
        }
        Some(v) if v.is_null => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_NULL;
            return;
        }
        None => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_NULL;
            return;
        }
        _ => {
            write_result(
                VdfReturn::error("from_string: expected STRING argument"),
                result,
            );
            return;
        }
    };

    // The server attaches any params it already knows to the binary result slot.
    // A non-empty set means the value came from a typed column. An empty set
    // (count == 0) means it's a bare constant whose params we must infer.
    let input = &result.__bindgen_anon_1.__bindgen_anon_2.type_params;
    let input_known = input.count > 0;
    let mut maybe = if input_known {
        let key = canonical_key(TypeParams { raw: input });
        MaybeParams::present(parse(Params::parse(&key)))
    } else {
        MaybeParams::empty()
    };

    // Run the author's encoder. When params were unknown, this is where it
    // inspects the string and fills them in via `set`.
    let ret = match encode(s, &mut maybe) {
        Ok(bytes) => VdfReturn::Binary(bytes),
        Err(e) => VdfReturn::error(e),
    };
    write_result(ret, result);

    // If the author inferred params (and the server gave us a buffer for them),
    // serialize them back so downstream operators see a fully-typed value.
    let succeeded = result.type_ == vef_return_value_type_t_VEF_RESULT_VALUE;
    if succeeded && !input_known && !result.out_type_params.is_null() {
        if let Some(p) = maybe.get() {
            let pairs = to_strings(p);
            let out = &mut *result.out_type_params;
            let buf: &mut [u8] = if out.buf.is_null() || out.max_buf_len == 0 {
                &mut []
            } else {
                std::slice::from_raw_parts_mut(out.buf.cast::<u8>(), out.max_buf_len)
            };
            let (needed, overflow) = write_inferred_params(buf, &pairs);
            out.actual_len = needed;
            out.overflow = overflow;
        }
    }
}

/// Dispatch a parameterized type's intrinsic-default VDF: compute the type's
/// default value (as a string) from its parameters. The server calls this once
/// per parameterized instantiation at DDL time, then encodes the string via
/// `from_string` and caches the result.
/// # Safety
/// `result` must be valid for the duration of the call.
pub unsafe fn dispatch_intrinsic_default_typed<P>(
    default_fn: fn(&P) -> Result<String, String>,
    parse: fn(Params) -> P,
    _args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let result = &mut *result;

    // The type params ride on the (binary) result slot, as with from_string.
    let params = {
        let raw = &result.__bindgen_anon_1.__bindgen_anon_2.type_params;
        parse(Params::parse(&canonical_key(TypeParams { raw })))
    };

    let ret = match default_fn(&params) {
        Ok(s) => VdfReturn::String(s),
        Err(e) => VdfReturn::error(e),
    };
    write_result(ret, result);
}

/// Copy `bytes` into the server's result buffer `buf` (capacity `max`) and mark
/// the result a VALUE. If it doesn't fit, set an error rather than truncate.
///
/// # Safety
/// `buf` must be writable for `max` bytes, and `result.error_msg` must be valid.
unsafe fn write_bytes_or_error(
    bytes: &[u8],
    buf: *mut u8,
    max: usize,
    result: &mut vef_vdf_result_t,
) {
    if bytes.len() <= max {
        // Fits the server-provided buffer.
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
        result.actual_len = bytes.len();
        result.type_ = vef_return_value_type_t_VEF_RESULT_VALUE;
    } else {
        // Too large for the result buffer. Errors rather than truncates.
        // The function should declare a larger buffer_size in func!.
        let msg = format!(
            "result of {} bytes exceeds the {max}-byte buffer. \
            Declare a larger buffer_size in func!",
            bytes.len()
        );
        result.type_ = vef_return_value_type_t_VEF_RESULT_ERROR;
        write_error_msg(msg.as_bytes(), result.error_msg);
    }
}

unsafe fn write_result(ret: VdfReturn, result: &mut vef_vdf_result_t) {
    match ret {
        VdfReturn::Null => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_NULL;
        }
        VdfReturn::String(s) => {
            let anon = &result.__bindgen_anon_1.__bindgen_anon_1;
            let (buf, max) = (anon.str_buf.cast::<u8>(), anon.max_str_len);
            write_bytes_or_error(s.as_bytes(), buf, max, result);
        }
        VdfReturn::Real(v) => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_VALUE;
            result.__bindgen_anon_1.real_value = v;
        }
        VdfReturn::Int(v) => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_VALUE;
            result.__bindgen_anon_1.int_value = v;
        }
        VdfReturn::Binary(bytes) => {
            let anon = &result.__bindgen_anon_1.__bindgen_anon_2;
            let (buf, max) = (anon.bin_buf, anon.max_bin_len);
            write_bytes_or_error(&bytes, buf, max, result);
        }
        VdfReturn::Warning(msg) => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_WARNING;
            write_error_msg(msg.as_bytes(), result.error_msg);
        }
        VdfReturn::Error(msg) => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_ERROR;
            write_error_msg(msg.as_bytes(), result.error_msg);
        }
    }
}

/// Serialize inferred type parameters into `buf` as canonical `key=value,...` (keys
/// sorted). snprintf-style: writes what fits and returns `(actual_len,
/// overflow)`, where `actual_len` is the full length needed and `overflow` is
/// true when it didn't all fit.
#[must_use]
pub fn write_inferred_params(buf: &mut [u8], pairs: &[(String, String)]) -> (usize, bool) {
    let joined = canonical_params_string(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let bytes = joined.as_bytes();
    let needed = bytes.len();
    let n = needed.min(buf.len());
    buf[..n].copy_from_slice(&bytes[..n]);
    (needed, needed > buf.len())
}

unsafe fn write_error_msg(msg: &[u8], buf: *mut c_char) {
    let max = (VEF_MAX_ERROR_LEN as usize).saturating_sub(1);
    let n = msg.len().min(max);
    std::ptr::copy_nonoverlapping(msg.as_ptr(), buf.cast::<u8>(), n);
    *buf.add(n) = 0;
}

/// Turn a C string pointer into a borrowed `&str`.
///
/// # Safety
/// `p` must point to a valid, NUL-terminated, UTF-8 string that lives for `'a`.
unsafe fn cstr_to_str<'a>(p: *const c_char) -> &'a str {
    std::str::from_utf8_unchecked(std::ffi::CStr::from_ptr(p).to_bytes())
}

unsafe fn build_func_ptr(d: &FuncDescriptor) -> *mut vef_func_desc_t {
    let (param_count, params_ptr) = if d.varargs {
        (villagesql_sys::VEF_PARAM_VARARGS, std::ptr::null())
    } else {
        let params: Box<[vef_type_t]> = d.params.iter().map(|t| t.to_raw()).collect();
        let count = u32::try_from(params.len()).expect("param count exceeds u32");
        (count, Box::into_raw(params) as *const vef_type_t)
    };
    let sig = Box::into_raw(Box::new(vef_signature_t {
        param_count,
        params: params_ptr,
        return_type: d.returns.to_raw(),
    }));
    Box::into_raw(Box::new(vef_func_desc_t {
        protocol: vef_protocol_t_VEF_PROTOCOL_3,
        name: d.sql_name,
        signature: sig,
        vdf: Some(d.trampoline),
        prerun: d.prerun,
        postrun: d.postrun,
        buffer_size: d.buffer_size,
        deterministic: d.deterministic,
        clear: d.clear,
        accumulate: d.accumulate,
    }))
}

/// Allocate a `vef_registration_t` from slices of descriptors.
///
/// # Panics
/// Panics if the number of functions or types exceeds `u32::MAX`.
///
/// # Safety
/// All descriptor fields must be valid for `'static`.
#[must_use]
pub unsafe fn build_registration(
    funcs: &[FuncDescriptor],
    types: &[TypeWithFuncs],
    caps: &[RequiredCapability],
) -> *mut vef_registration_t {
    // ── Functions: explicit + embedded from each type ─────────────────────────
    let mut func_ptrs: Vec<*mut vef_func_desc_t> = Vec::new();
    for d in funcs {
        func_ptrs.push(build_func_ptr(d));
    }
    for t in types {
        for d in &t.embedded_funcs {
            func_ptrs.push(build_func_ptr(d));
        }
    }
    let func_count = u32::try_from(func_ptrs.len()).expect("func count exceeds u32");
    let funcs_ptr = Box::into_raw(func_ptrs.into_boxed_slice()).cast::<*mut vef_func_desc_t>();

    // ── Types ──────────────────────────────────────────────────────────────────
    let mut type_ptrs: Vec<*mut vef_type_desc_t> = Vec::with_capacity(types.len());
    for t in types {
        type_ptrs.push(Box::into_raw(Box::new(vef_type_desc_t {
            protocol: vef_protocol_t_VEF_PROTOCOL_3,
            name: t.descriptor.sql_name,
            persisted_length: t.descriptor.persisted_length,
            max_decode_buffer_length: t.descriptor.max_decode_buffer_length,
            encode_func: None,
            decode_func: None,
            compare_func: None,
            hash_func: None,
            encode_vdf_name: t.descriptor.encode_vdf_name,
            decode_vdf_name: t.descriptor.decode_vdf_name,
            compare_vdf_name: t.descriptor.compare_vdf_name,
            hash_vdf_name: t.descriptor.hash_vdf_name,
            int_to_params_vdf_name: t.descriptor.int_to_params_vdf_name,
            resolve_params_vdf_name: t.descriptor.resolve_params_vdf_name,
            intrinsic_default_vdf_name: t.descriptor.intrinsic_default_vdf_name,
            intrinsic_default_str: t.descriptor.intrinsic_default_str,
            max_persisted_length: t.descriptor.max_persisted_length,
        })));
    }
    let type_count = u32::try_from(type_ptrs.len()).expect("type count exceeds u32");
    let types_ptr = Box::into_raw(type_ptrs.into_boxed_slice()).cast::<*mut vef_type_desc_t>();
    let cap_raws: Box<[vef_required_capability_t]> =
        caps.iter().map(RequiredCapability::to_raw).collect();
    let cap_count = u32::try_from(cap_raws.len()).expect("capability count exceeds u32");
    let caps_ptr: *const vef_required_capability_t =
        Box::into_raw(cap_raws).cast::<vef_required_capability_t>();

    Box::into_raw(Box::new(vef_registration_t {
        protocol: vef_protocol_t_VEF_PROTOCOL_3,
        error_msg: std::ptr::null_mut(),
        deprecated_extension_version: std::ptr::null(),
        sdk_version: vef_version_t {
            major: 0,
            minor: 0,
            patch: 1,
            extra: std::ptr::null(),
        },
        deprecated_extension_name: std::ptr::null(),
        func_count,
        funcs: funcs_ptr,
        type_count,
        types: types_ptr,
        required_capability_count: cap_count,
        required_capabilities: caps_ptr,
    }))
}

/// Free all memory allocated by [`build_registration`].
///
/// # Safety
/// `registration` must have been returned by [`build_registration`].
pub unsafe fn free_registration(registration: *mut vef_registration_t) {
    if registration.is_null() {
        return;
    }
    let reg = Box::from_raw(registration);

    // Free functions.
    let funcs = std::slice::from_raw_parts_mut(reg.funcs, reg.func_count as usize);
    for &func_ptr in funcs.iter() {
        let func = Box::from_raw(func_ptr);
        let sig = Box::from_raw(func.signature);
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            sig.params.cast_mut(),
            sig.param_count as usize,
        )));
        drop(sig);
        drop(func);
    }
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        reg.funcs,
        reg.func_count as usize,
    )));

    // Free types.
    let types = std::slice::from_raw_parts_mut(reg.types, reg.type_count as usize);
    for &type_ptr in types.iter() {
        drop(Box::from_raw(type_ptr));
    }
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        reg.types,
        reg.type_count as usize,
    )));

    // Free required capabilities.
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        reg.required_capabilities.cast_mut(),
        reg.required_capability_count as usize,
    )));
    drop(reg);
}

// ── Macros ────────────────────────────────────────────────────────────────────

/// Produce a [`Type::Custom`] value for the named custom type.
///
/// ```ignore
/// villagesql::custom!("rational")   // → Type::Custom pointing to b"rational\0"
/// ```
#[macro_export]
macro_rules! custom {
    ($name:literal) => {
        $crate::Type::Custom(concat!($name, "\0").as_bytes().as_ptr() as *const ::std::ffi::c_char)
    };
}

/// Declare a `VillageSQL` extension and generate the `vef_register` /
/// `vef_unregister` C entry points.
///
/// ```ignore
/// villagesql::extension! {
///     funcs: [
///         villagesql::func!(my_impl, "sql_name", [villagesql::Type::String] -> villagesql::Type::String),
///     ],
///     types: [
///         villagesql::custom_type!(
///             type_name: "my_type",
///             persisted_length: 8,
///             max_decode_buffer_length: 32,
///             encode: my_encode,
///             decode: my_decode,
///             compare: my_compare,
///         ),
///     ]
/// }
/// ```
///
/// The `types:` list is optional; omitting it is equivalent to `types: []`.
#[macro_export]
macro_rules! extension {
    // Canonical form: funcs + types + requires.
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ],
        types: [ $( $type_desc:expr ),* $(,)? ],
        requires: [ $( $cap:expr ),* $(,)? ] $(,)?
    ) => {
        #[no_mangle]
        pub unsafe extern "C" fn vef_register(
            _arg: *const $crate::sys::vef_register_arg_t,
        ) -> *mut $crate::sys::vef_registration_t {
            let funcs: &[$crate::FuncDescriptor] = &[$($func_desc),*];
            let types: ::std::vec::Vec<$crate::TypeWithFuncs> = vec![$($type_desc),*];
            let caps: &[$crate::preview::RequiredCapability] = &[$($crate::preview::Capability::request($cap)),*];
            $crate::build_registration(funcs, &types, caps)
        }

        #[no_mangle]
        pub unsafe extern "C" fn vef_unregister(
            _arg: *const $crate::sys::vef_unregister_arg_t,
            registration: *mut $crate::sys::vef_registration_t,
        ) {
            $crate::free_registration(registration);
        }
    };

    // funcs + types (no requires).
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ],
        types: [ $( $type_desc:expr ),* $(,)? ] $(,)?
    ) => {
        $crate::extension! {
            funcs: [ $($func_desc),* ],
            types: [ $($type_desc),* ],
            requires: []
        }
    };

    // funcs + requires (no custom types).
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ],
        requires: [ $( $cap:expr ),* $(,)? ] $(,)?
    ) => {
        $crate::extension! {
            funcs: [ $($func_desc),* ],
            types: [],
            requires: [ $($cap),* ]
        }
    };

    // funcs only (backward compatible).
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ] $(,)?
    ) => {
        $crate::extension! {
            funcs: [ $($func_desc),* ],
            types: [],
            requires: []
        }
    };
}

/// Build a [`FuncDescriptor`] for one VDF and generate its `extern "C"` trampoline.
///
/// ```ignore
/// villagesql::func!(impl_fn, "sql_name", [villagesql::Type::String] -> villagesql::Type::String)
/// villagesql::func!(impl_fn, "sql_name", [villagesql::custom!("my_type")] -> villagesql::custom!("my_type"),
///             deterministic: true)
/// ```
#[macro_export]
macro_rules! func {
    // Prerun form: a function that owns per-statement state (setup in prerun,
    // auto-freed in postrun). `state:` names the state type. `prerun:` names the setup fn.
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     state: $state:ty,  prerun: $prerun:ident, buffer_size: $bs:expr, deterministic: $det:expr) => {{
        $crate::paste::paste! {
            // row trampoline borrows the state.
            unsafe extern "C" fn [< __vsql_trampoline_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf_with_state::<$state>($impl_fn, args, result);
            }
            // prerun trampoline
            unsafe extern "C" fn [< __vsql_prerun_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_prerun_args_t,
                result: *mut $crate::sys::vef_prerun_result_t,
            ) {
                $crate::dispatch_prerun::<$state>($prerun, args, result);
            }
            // postrun trampoline. auto-drops the state.
            unsafe extern "C" fn [< __vsql_postrun_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_postrun_args_t,
                _result: *mut $crate::sys::vef_postrun_result_t,
            ) {
                $crate::dispatch_postrun::<$state>(args);
            }
            static [< __VSQL_PARAMS_ $impl_fn:upper >]: &[$crate::Type] = &[$($param),*];
            $crate::FuncDescriptor {
                sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_PARAMS_ $impl_fn:upper >],
                returns: $ret,
                trampoline: [< __vsql_trampoline_ $impl_fn >],
                buffer_size: $bs,
                deterministic: $det,
                varargs: false,
                prerun: Some([< __vsql_prerun_ $impl_fn >]),
                postrun: Some([< __vsql_postrun_ $impl_fn >]),
                clear: None,
                accumulate: None,
            }
        }
     }};

    // Prerun form, defaults: server buffer size, non-deterministic.
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     state: $state:ty, prerun: $prerun:ident) => {
        $crate::func!($impl_fn, $sql_name, [$($param),*] -> $ret,
            state: $state, prerun: $prerun, buffer_size: 0, deterministic: false)
    };
    // Full form: declare the result-buffer size and determinism. `buffer_size`
    // is a plain value (ideally computed from data, not a magic literal); 0
    // uses the server default (256 bytes).
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     buffer_size: $bs:expr, deterministic: $det:expr) => {{
        $crate::paste::paste! {
            unsafe extern "C" fn [< __vsql_trampoline_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf($impl_fn, args, result);
            }
            static [< __VSQL_PARAMS_ $impl_fn:upper >]: &[$crate::Type] = &[$($param),*];
            $crate::FuncDescriptor {
                sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_PARAMS_ $impl_fn:upper >],
                returns: $ret,
                trampoline: [< __vsql_trampoline_ $impl_fn >],
                buffer_size: $bs,
                deterministic: $det,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            }
        }
    }};
    // buffer_size only (determinism defaults to false).
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     buffer_size: $bs:expr) => {
        $crate::func!($impl_fn, $sql_name, [$($param),*] -> $ret,
            buffer_size: $bs, deterministic: false)
    };
    // deterministic only (buffer_size defaults to 0 = server default).
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     deterministic: $det:expr) => {
        $crate::func!($impl_fn, $sql_name, [$($param),*] -> $ret,
            buffer_size: 0, deterministic: $det)
    };
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr) => {
        $crate::func!($impl_fn, $sql_name, [$($param),*] -> $ret,
            buffer_size: 0, deterministic: false)
    };
}

/// Declare an aggregate SQL function (SUM/COUNT-style). The first ident is the
/// result fn (`fn(&State) -> VdfReturn`), which produces each group's value.
/// `state:` names the accumulator (must implement `Default`). `clear:` resets
/// it at the start of each group. `accumulate:` folds one row.
///
/// ```ignore
/// villagesql::agg_func!(my_sum_result, "my_sum", [villagesql::Type::Int] -> villagesql::Type::Int,
///     state: SumState, clear: my_sum_clear, accumulate: my_sum_acc)
/// ```
#[macro_export]
macro_rules! agg_func {
    // Full form: result-buffer size and determinism specified.
    ($result_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     state: $state:ty, clear: $clear:ident, accumulate: $accum:ident,
     buffer_size: $bs:expr, deterministic: $det:expr) => {{
        $crate::paste::paste! {
            // result trampoline: occupies the `vdf` slot. Called once per group.
            unsafe extern "C" fn [< __vsql_trampoline_ $result_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_agg_result::<$state>($result_fn, args, result);
            }
            // prerun trampoline: allocates State::default().
            unsafe extern "C" fn [< __vsql_agg_prerun_ $result_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_prerun_args_t,
                result: *mut $crate::sys::vef_prerun_result_t,
            ) {
                $crate::dispatch_agg_prerun::<$state>(args, result);
            }
            // postrun trampoline: drops the state.
            unsafe extern "C" fn [< __vsql_postrun_ $result_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_postrun_args_t,
                _result: *mut $crate::sys::vef_postrun_result_t,
            ) {
                $crate::dispatch_postrun::<$state>(args);
            }
            // clear trampoline: note the ABI has no result param here.
            unsafe extern "C" fn [< __vsql_clear_ $result_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
            ) {
                $crate::dispatch_clear::<$state>($clear, args);
            }
            // accumulate trampoline: result param unused (server pre-sets VALUE).
            unsafe extern "C" fn [< __vsql_accumulate_ $result_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                _result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_accumulate::<$state>($accum, args);
            }
            static [< __VSQL_PARAMS_ $result_fn:upper >]: &[$crate::Type] = &[$($param),*];
            $crate::FuncDescriptor {
                sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_PARAMS_ $result_fn:upper >],
                returns: $ret,
                trampoline: [< __vsql_trampoline_ $result_fn >],
                buffer_size: $bs,
                deterministic: $det,
                varargs: false,
                prerun: Some([< __vsql_agg_prerun_ $result_fn >]),
                postrun: Some([< __vsql_postrun_ $result_fn >]),
                clear: Some([< __vsql_clear_ $result_fn >]),
                accumulate: Some([< __vsql_accumulate_ $result_fn >]),
            }
        }
    }};

    // Shorthand: server-default buffer size, non-deterministic.
    ($result_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     state: $state:ty, clear: $clear:ident, accumulate: $accum:ident) => {
        $crate::agg_func!($result_fn, $sql_name, [$($param),*] -> $ret,
            state: $state, clear: $clear, accumulate: $accum,
            buffer_size: 0, deterministic: false)
    };
}

/// Declare a varargs VDF. The function accepts any number of arguments. The
/// `prerun` hook is responsible for validating the count and types
/// (the server does none). `state:` names the per-statement state. `prerun:`
/// sets it up.
#[macro_export]
macro_rules! varargs_func {
    // Full form: buffer size + determinism specified.
    ($impl_fn:ident, $sql_name:literal -> $ret:expr,
     state: $state:ty, prerun: $prerun:ident, buffer_size: $bs:expr, deterministic: $det:expr) => {{
        $crate::paste::paste! {
            // row trampoline borrows the state.
            unsafe extern "C" fn [< __vsql_trampoline_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf_with_state::<$state>($impl_fn, args, result);
            }
            // prerun trampoline: validates arg count/types, sets up state.
            unsafe extern "C" fn [< __vsql_prerun_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_prerun_args_t,
                result: *mut $crate::sys::vef_prerun_result_t,
            ) {
                $crate::dispatch_prerun::<$state>($prerun, args, result);
            }
            // postrun trampoline: auto-drops the state.
            unsafe extern "C" fn [< __vsql_postrun_ $impl_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_postrun_args_t,
                _result: *mut $crate::sys::vef_postrun_result_t,
            ) {
                $crate::dispatch_postrun::<$state>(args);
            }
            $crate::FuncDescriptor {
                sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: &[],
                returns: $ret,
                trampoline: [< __vsql_trampoline_ $impl_fn >],
                buffer_size: $bs,
                deterministic: $det,
                varargs: true,
                prerun: Some([< __vsql_prerun_ $impl_fn >]),
                postrun: Some([< __vsql_postrun_ $impl_fn >]),
                clear: None,
                accumulate: None,
            }
        }
    }};

    // Shorthand: server-default buffer size, non-deterministic.
    ($impl_fn:ident, $sql_name:literal -> $ret:expr,
     state: $state:ty, prerun: $prerun:ident) => {
        $crate::varargs_func!($impl_fn, $sql_name -> $ret,
            state: $state, prerun: $prerun, buffer_size: 0, deterministic: false)
    };

    // Prerun-only form: validatoin (+ optional buffer sizing), no state.
    ($impl_fn:ident, $sql_name:literal -> $ret:expr,
        prerun: $prerun:ident, buffer_size: $bs:expr, deterministic: $det:expr) => {{
            $crate::paste::paste! {
                // row trampoline: STATELESS. dispatch_vdf, not _with_state.
                unsafe extern "C" fn [< __vsql_trampoline_ $impl_fn >](
                    _ctx: *mut $crate::sys::vef_context_t,
                    args: *mut $crate::sys::vef_vdf_args_t,
                    result: *mut $crate::sys::vef_vdf_result_t,
                ) {
                    $crate::dispatch_vdf($impl_fn, args, result);
                }
                // prerun trampoline: validates only. State type is (). Nothing stored.
                unsafe extern "C" fn [< __vsql_prerun_ $impl_fn >](
                    _ctx: *mut $crate::sys::vef_context_t,
                    args: *mut $crate::sys::vef_prerun_args_t,
                    result: *mut $crate::sys::vef_prerun_result_t,
                ) {
                    $crate::dispatch_prerun::<()>($prerun, args, result);
                }
                $crate::FuncDescriptor {
                    sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                        as *const ::std::os::raw::c_char,
                    params: &[],
                    returns: $ret,
                    trampoline: [< __vsql_trampoline_ $impl_fn >],
                    buffer_size: $bs,
                    deterministic: $det,
                    varargs: true,
                    prerun: Some([< __vsql_prerun_ $impl_fn >]),
                    postrun: None,
                    clear: None,
                    accumulate: None,
                }
            }
        }};

        // Prerun-only shorthand.
        ($impl_fn:ident, $sql_name:literal -> $ret:expr, prerun: $prerun:ident) => {
            $crate::varargs_func!($impl_fn, $sql_name -> $ret,
                prerun: $prerun, buffer_size: 0, deterministic: false)
        };

        // Bare form: no prerun, no state, no validation (server-default buffer).
        ($impl_fn: ident, $sql_name:literal -> $ret:expr,
            buffer_size: $bs:expr, deterministic: $det:expr) => {{
                $crate::paste::paste! {
                    unsafe extern "C" fn [< __vsql_trampoline_ $impl_fn >](
                        _ctx: *mut $crate::sys::vef_context_t,
                        args: *mut $crate::sys::vef_vdf_args_t,
                        result: *mut $crate::sys::vef_vdf_result_t,
                    ) {
                        $crate::dispatch_vdf($impl_fn, args, result);
                    }
                    $crate::FuncDescriptor {
                        sql_name: concat!($sql_name, "\0").as_bytes().as_ptr()
                            as *const ::std::os::raw::c_char,
                        params: &[],
                        returns: $ret,
                        trampoline: [< __vsql_trampoline_ $impl_fn >],
                        buffer_size: $bs,
                        deterministic: $det,
                        varargs: true,
                        prerun: None,
                        postrun: None,
                        clear: None,
                        accumulate: None,
                    }
                }
            }};

            // Bare shorthand.
            ($impl_fn:ident, $sql_name:literal -> $ret:expr) => {
                $crate::varargs_func!($impl_fn, $sql_name -> $ret,
                    buffer_size: 0, deterministic: false)
            };
}

/// Internal: emits the four shared VDFs (`from_string`/`to_string`/`compare`/`hash`)
/// for a custom type and evaluates to a `Vec<FuncDescriptor>` registering them.
/// Not part of the public API — called by `custom_type!` and `parameterized_type!`.
#[macro_export]
#[doc(hidden)]
macro_rules! __vsql_type_vdfs {
    (
        type_name: $type_name:literal,
        encode: $enc_fn:ident,
        decode: $dec_fn:ident,
        compare: $cmp_fn:ident
        $(, hash: $hash_fn:ident)?
        $(,)?
    ) => {{
        $crate::paste::paste! {
            // TYPE::from_string(STRING) -> CUSTOM
            fn [< __vsql_from_string_vdf_ $enc_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match args.get(0) {
                    Some($crate::InValue::String(s)) => match $enc_fn(s) {
                        Ok(bytes) => $crate::VdfReturn::Binary(bytes),
                        Err(e) => $crate::VdfReturn::error(e),
                    },
                    Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                    _ => $crate::VdfReturn::error(
                        concat!($type_name, "::from_string: expected STRING argument"),
                    ),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_from_string_ $enc_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_from_string_vdf_ $enc_fn >], args, result);
            }
            static [< __VSQL_FROM_STRING_PARAMS_ $enc_fn:upper >]: &[$crate::Type] =
                &[$crate::Type::String];

            // TYPE::to_string(CUSTOM) -> STRING
            fn [< __vsql_to_string_vdf_ $dec_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match args.get(0) {
                    Some($crate::InValue::Custom(b)) => match $dec_fn(b) {
                        Ok(s) => $crate::VdfReturn::String(s),
                        Err(e) => $crate::VdfReturn::error(e),
                    },
                    Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                    _ => $crate::VdfReturn::error(
                        concat!($type_name, "::to_string: expected CUSTOM argument"),
                    ),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_to_string_ $dec_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_to_string_vdf_ $dec_fn >], args, result);
            }
            static [< __VSQL_TO_STRING_PARAMS_ $dec_fn:upper >]: &[$crate::Type] =
                &[$crate::custom!($type_name)];

            // TYPE::compare(CUSTOM, CUSTOM) -> INT
            fn [< __vsql_compare_vdf_ $cmp_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match (args.get(0), args.get(1)) {
                    (Some($crate::InValue::Custom(a)), Some($crate::InValue::Custom(b))) => {
                        $crate::VdfReturn::Int(match $cmp_fn(a, b) {
                            ::std::cmp::Ordering::Less => -1,
                            ::std::cmp::Ordering::Equal => 0,
                            ::std::cmp::Ordering::Greater => 1,
                        })
                    }
                    _ => $crate::VdfReturn::null(),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_compare_ $cmp_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_compare_vdf_ $cmp_fn >], args, result);
            }
            static [< __VSQL_COMPARE_PARAMS_ $cmp_fn:upper >]: &[$crate::Type] = &[
                $crate::custom!($type_name),
                $crate::custom!($type_name),
            ];

            // TYPE::hash(CUSTOM) -> INT (optional)
            $(
                fn [< __vsql_hash_vdf_ $hash_fn >](
                    args: &[$crate::InValue],
                ) -> $crate::VdfReturn {
                    match args.get(0) {
                        Some($crate::InValue::Custom(b)) => {
                            $crate::VdfReturn::Int($hash_fn(b) as i64)
                        }
                        Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                        _ => $crate::VdfReturn::error(
                            concat!($type_name, "::hash: expected CUSTOM argument"),
                        ),
                    }
                }
                unsafe extern "C" fn [< __vsql_trampoline_hash_ $hash_fn >](
                    _ctx: *mut $crate::sys::vef_context_t,
                    args: *mut $crate::sys::vef_vdf_args_t,
                    result: *mut $crate::sys::vef_vdf_result_t,
                ) {
                    $crate::dispatch_vdf([< __vsql_hash_vdf_ $hash_fn >], args, result);
                }
                static [< __VSQL_HASH_PARAMS_ $hash_fn:upper >]: &[$crate::Type] =
                    &[$crate::custom!($type_name)];
            )?

            // Register them, evaluate to the Vec
            #[allow(unused_mut)]
            let mut __embedded: ::std::vec::Vec<$crate::FuncDescriptor> =
                ::std::vec::Vec::new();
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::from_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_FROM_STRING_PARAMS_ $enc_fn:upper >],
                returns: $crate::custom!($type_name),
                trampoline: [< __vsql_trampoline_from_string_ $enc_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::to_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_TO_STRING_PARAMS_ $dec_fn:upper >],
                returns: $crate::Type::String,
                trampoline: [< __vsql_trampoline_to_string_ $dec_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::compare\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_COMPARE_PARAMS_ $cmp_fn:upper >],
                returns: $crate::Type::Int,
                trampoline: [< __vsql_trampoline_compare_ $cmp_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            $(
                __embedded.push($crate::FuncDescriptor {
                    sql_name: concat!($type_name, "::hash\0").as_bytes().as_ptr()
                        as *const ::std::os::raw::c_char,
                    params: [< __VSQL_HASH_PARAMS_ $hash_fn:upper >],
                    returns: $crate::Type::Int,
                    trampoline: [< __vsql_trampoline_hash_ $hash_fn >],
                    buffer_size: 0,
                    deterministic: true,
                    varargs: false,
                    prerun: None,
                    postrun: None,
                    clear: None,
                    accumulate: None,
                });
            )?

            __embedded
        }
    }};
}

/// Internal: like `__vsql_type_vdfs!` but for parameterized types with a typed
/// params struct `P`. `to_string`/`compare`/`hash` receive `&P` from a per-type
/// `TypeParamsCache<P>` (defined here). `from_string` stays untyped (no params
/// on its STRING input). Not part of the public API.
#[macro_export]
#[doc(hidden)]
macro_rules! __vsql_type_vdfs_typed {
    (
        type_name: $type_name:literal,
        encode: $enc_fn:ident,
        decode: $dec_fn:ident,
        compare: $cmp_fn:ident,
        int_to_params: $i2p_fn:ident,
        resolve_params: $rp_fn:ident,
        max_decode_buffer_length: $max_dec:expr,
        params_type: $p_ty:ty,
        params_parse: $parse_fn:ident,
        params_to_strings: $to_strings_fn:ident
        $(, hash: $hash_fn:ident)?
        $(, intrinsic_default_fn: $default_fn:ident)?
        $(,)?
    ) => {{
        $crate::paste::paste! {
            // Per-type params cache: parses P once per distinct params string
            static [< __VSQL_PARAMS_CACHE_$dec_fn:upper >]:
                $crate::TypeParamsCache<$p_ty> = $crate::TypeParamsCache::new();

            // TYPE::from_string(STRING) -> CUSTOM (typed; may infer params)
            unsafe extern "C" fn [< __vsql_trampoline_from_string_ $enc_fn >] (
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_from_string_typed(
                    $enc_fn, $parse_fn, $to_strings_fn, args, result
                );
            }
            static [< __VSQL_FROM_STRING_PARAMS_ $enc_fn:upper >]: &[$crate::Type] =
                &[$crate::Type::String];

            // TYPE::to_string(CUSTOM) -> STRING (typed)
            fn [< __vsql_to_string_vdf_ $dec_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match args.get(0) {
                    Some($crate::InValue::CustomWithParams{ bytes: b, params: tp }) => {
                        let p = [< __VSQL_PARAMS_CACHE_ $dec_fn:upper >].get(*tp, $parse_fn);
                        match $dec_fn(b, p) {
                            Ok(s) => $crate::VdfReturn::String(s),
                            Err(e) => $crate::VdfReturn::error(e),
                        }
                    }
                    Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                    _ => $crate::VdfReturn::error(
                        concat!($type_name, "::to_string: expected CUSTOM argument"),
                    ),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_to_string_ $dec_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_to_string_vdf_ $dec_fn >], args, result);
            }
            static [< __VSQL_TO_STRING_PARAMS_ $dec_fn:upper >]: &[$crate::Type] =
                &[$crate::custom!($type_name)];

            // TYPE::compare(CUSTOM, CUSTOM) -> INT (typed)
            fn [< __vsql_compare_vdf_ $cmp_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match (args.get(0), args.get(1)) {
                    (
                        Some($crate::InValue::CustomWithParams{ bytes: a, params: tp }),
                        Some($crate::InValue::CustomWithParams{ bytes: b, .. }),
                    ) => {
                        let p = [< __VSQL_PARAMS_CACHE_ $dec_fn:upper >].get(*tp, $parse_fn);
                        $crate::VdfReturn::Int(match $cmp_fn(a, b, p) {
                            ::std::cmp::Ordering::Less => -1,
                            ::std::cmp::Ordering::Equal => 0,
                            ::std::cmp::Ordering::Greater => 1,
                        })
                    }
                    _ => $crate::VdfReturn::null(),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_compare_ $cmp_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_compare_vdf_ $cmp_fn >], args, result);
            }
            static [< __VSQL_COMPARE_PARAMS_ $cmp_fn:upper >]: &[$crate::Type] = &[
                $crate::custom!($type_name),
                $crate::custom!($type_name),
            ];

            // TYPE::hash(CUSTOM) -> INT (optional, typed)
            $(
                fn [< __vsql_hash_vdf_ $hash_fn >](
                    args: &[$crate::InValue],
                ) -> $crate::VdfReturn {
                    match args.get(0) {
                        Some($crate::InValue::CustomWithParams{ bytes: b, params: tp }) => {
                            let p = [< __VSQL_PARAMS_CACHE_ $dec_fn:upper >].get(*tp, $parse_fn);
                            $crate::VdfReturn::Int($hash_fn(b, p) as i64)
                        }
                        Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                        _ => $crate::VdfReturn::error(
                            concat!($type_name, "::hash: expected CUSTOM argument"),
                        ),
                    }
                }
                unsafe extern "C" fn [< __vsql_trampoline_hash_ $hash_fn >](
                    _ctx: *mut $crate::sys::vef_context_t,
                    args: *mut $crate::sys::vef_vdf_args_t,
                    result: *mut $crate::sys::vef_vdf_result_t,
                ) {
                    $crate::dispatch_vdf([< __vsql_hash_vdf_ $hash_fn >], args, result);
                }
                static [< __VSQL_HASH_PARAMS_ $hash_fn:upper >]: &[$crate::Type] =
                    &[$crate::custom!($type_name)];
            )?

            // TYPE::int_to_params(INT) -> STRING
            fn [< __vsql_int_to_params_vdf_ $i2p_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match args.get(0) {
                    Some($crate::InValue::Int(n)) => match $i2p_fn(*n) {
                        Ok(s) => $crate::VdfReturn::String(s),
                        Err(e) => $crate::VdfReturn::error(e),
                    },
                    Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                    _ => $crate::VdfReturn::error(
                        concat!($type_name, "::int_to_params: expected INT argument"),
                    ),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_int_to_params_ $i2p_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_int_to_params_vdf_ $i2p_fn >], args, result);
            }
            static [< __VSQL_INT_TO_PARAMS_PARAMS_ $i2p_fn:upper >]: &[$crate::Type] =
                &[$crate::Type::Int];

            // TYPE::resolve_params(STRING) -> STRING
            fn [< __vsql_resolve_params_vdf_ $rp_fn >](
                args: &[$crate::InValue],
            ) -> $crate::VdfReturn {
                match args.get(0) {
                    Some($crate::InValue::String(s)) => {
                        match $rp_fn($crate::Params::parse(s)) {
                            Ok(resolved) => $crate::VdfReturn::String(resolved.to_wire_string()),
                            Err(e) => $crate::VdfReturn::error(e),
                        }
                    }
                    Some($crate::InValue::Null) | None => $crate::VdfReturn::null(),
                    _ => $crate::VdfReturn::error(
                        concat!($type_name, "::resolve_params: expected STRING argument"),
                    ),
                }
            }
            unsafe extern "C" fn [< __vsql_trampoline_resolve_params_ $rp_fn >](
                _ctx: *mut $crate::sys::vef_context_t,
                args: *mut $crate::sys::vef_vdf_args_t,
                result: *mut $crate::sys::vef_vdf_result_t,
            ) {
                $crate::dispatch_vdf([< __vsql_resolve_params_vdf_ $rp_fn >], args, result);
            }
            static [< __VSQL_RESOLVE_PARAMS_PARAMS_ $rp_fn:upper >]: &[$crate::Type] =
                &[$crate::Type::String];

            // TYPE::intrinsic_default() -> STRING (optional)
            $(
                unsafe extern "C" fn [< __vsql_trampoline_intrinsic_default_ $default_fn >](
                    _ctx: *mut $crate::sys::vef_context_t,
                    args: *mut $crate::sys::vef_vdf_args_t,
                    result: *mut $crate::sys::vef_vdf_result_t,
                ) {
                    $crate::dispatch_intrinsic_default_typed($default_fn, $parse_fn, args, result);
                }
                static [< __VSQL_INTRINSIC_DEFAULT_PARAMS_ $default_fn:upper >]: &[$crate::Type] = &[];
            )?

            // Register them, evaluate to the Vec
            #[allow(unused_mut)]
            let mut __embedded: ::std::vec::Vec<$crate::FuncDescriptor> =
                ::std::vec::Vec::new();
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::from_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_FROM_STRING_PARAMS_ $enc_fn:upper >],
                returns: $crate::custom!($type_name),
                trampoline: [< __vsql_trampoline_from_string_ $enc_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::to_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_TO_STRING_PARAMS_ $dec_fn:upper >],
                returns: $crate::Type::String,
                trampoline: [< __vsql_trampoline_to_string_ $dec_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::compare\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_COMPARE_PARAMS_ $cmp_fn:upper >],
                returns: $crate::Type::Int,
                trampoline: [< __vsql_trampoline_compare_ $cmp_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            $(
                __embedded.push($crate::FuncDescriptor {
                    sql_name: concat!($type_name, "::hash\0").as_bytes().as_ptr()
                        as *const ::std::os::raw::c_char,
                    params: [< __VSQL_HASH_PARAMS_ $hash_fn:upper >],
                    returns: $crate::Type::Int,
                    trampoline: [< __vsql_trampoline_hash_ $hash_fn >],
                    buffer_size: 0,
                    deterministic: true,
                    varargs: false,
                    prerun: None,
                    postrun: None,
                    clear: None,
                    accumulate: None,
                });
            )?
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::int_to_params\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_INT_TO_PARAMS_PARAMS_ $i2p_fn:upper >],
                returns: $crate::Type::String,
                trampoline: [< __vsql_trampoline_int_to_params_ $i2p_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::resolve_params\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_RESOLVE_PARAMS_PARAMS_ $rp_fn:upper >],
                returns: $crate::Type::String,
                trampoline: [< __vsql_trampoline_resolve_params_ $rp_fn >],
                buffer_size: 0,
                deterministic: true,
                varargs: false,
                prerun: None,
                postrun: None,
                clear: None,
                accumulate: None,
            });
            $(
                __embedded.push($crate::FuncDescriptor {
                    sql_name: concat!($type_name, "::intrinsic_default\0").as_bytes().as_ptr()
                        as *const ::std::os::raw::c_char,
                    params: [< __VSQL_INTRINSIC_DEFAULT_PARAMS_ $default_fn:upper >],
                    returns: $crate::Type::String,
                    trampoline: [< __vsql_trampoline_intrinsic_default_ $default_fn >],
                    buffer_size: $max_dec,
                    deterministic: true,
                    varargs: false,
                    prerun: None,
                    postrun: None,
                    clear: None,
                    accumulate: None,
                });
            )?

            __embedded
        }
    }};
}

#[macro_export]
macro_rules! custom_type {
    (
        type_name: $type_name:literal,
        persisted_length: $plen:expr,
        max_decode_buffer_length: $max_dec:expr,
        encode: $enc_fn:ident,
        decode: $dec_fn:ident,
        compare: $cmp_fn:ident
        $(, hash: $hash_fn:ident)?
        $(, default: $default_str:literal)?
        $(,)?
    ) => {{
        let __embedded = $crate::__vsql_type_vdfs!(
            type_name: $type_name,
            encode: $enc_fn,
            decode: $dec_fn,
            compare: $cmp_fn
            $(, hash: $hash_fn)?
        );

        #[allow(unused_mut)]
        let mut __default: *const ::std::ffi::c_char = ::std::ptr::null();
        $( __default = concat!($default_str, "\0").as_bytes().as_ptr()
            as *const ::std::ffi::c_char; )?

        #[allow(unused_mut)]
        let mut __hash_vdf_name: *const ::std::ffi::c_char = ::std::ptr::null();
        $( let _ = stringify!($hash_fn);
           __hash_vdf_name = concat!($type_name, "::hash\0").as_bytes().as_ptr()
            as *const ::std::ffi::c_char; )?

        $crate::TypeWithFuncs {
            descriptor: $crate::TypeDescriptor {
                sql_name: concat!($type_name, "\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                persisted_length: $plen,
                max_decode_buffer_length: $max_dec,
                encode_vdf_name: concat!($type_name, "::from_string\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                decode_vdf_name: concat!($type_name, "::to_string\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                compare_vdf_name: concat!($type_name, "::compare\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                hash_vdf_name: __hash_vdf_name,
                int_to_params_vdf_name: ::std::ptr::null(),
                resolve_params_vdf_name: ::std::ptr::null(),
                intrinsic_default_str: __default,
                intrinsic_default_vdf_name: ::std::ptr::null(),
                max_persisted_length: 0,
            },
            embedded_funcs: __embedded,
        }
    }};
}

/// Defines a parameterized custom SQL type. Like [`custom_type!`], but the
/// persisted length is computed per-column from type parameters via
/// `int_to_params` / `resolve_params` instead of being a fixed constant.
#[macro_export]
macro_rules! parameterized_type {
    (
        type_name: $type_name:literal,
        max_persisted_length: $max_plen:expr,
        max_decode_buffer_length: $max_dec:expr,
        encode: $enc_fn:ident,
        decode: $dec_fn:ident,
        compare: $cmp_fn:ident,
        int_to_params: $i2p_fn:ident,
        resolve_params: $rp_fn:ident,
        params_type: $p_ty:ty,
        params_parse: $parse_fn:ident,
        params_to_strings: $to_strings_fn:ident
        $(, hash: $hash_fn:ident)?
        $(, default: $default_str:literal)?
        $(, intrinsic_default_fn: $default_fn:ident)?
        $(,)?
    ) => {{
        // Stamp all VDFs (from_string/to_string/compare/hash + int_to_params/
        // resolve_params/intrinsic_default) and collect their descriptors.
        let mut __embedded = $crate::__vsql_type_vdfs_typed!(
            type_name: $type_name,
            encode: $enc_fn,
            decode: $dec_fn,
            compare: $cmp_fn,
            int_to_params: $i2p_fn,
            resolve_params: $rp_fn,
            max_decode_buffer_length: $max_dec,
            params_type: $p_ty,
            params_parse: $parse_fn,
            params_to_strings: $to_strings_fn
            $(, hash: $hash_fn)?
            $(, intrinsic_default_fn: $default_fn)?
        );

        #[allow(unused_mut)]
        let mut __default: *const ::std::ffi::c_char = ::std::ptr::null();
        $( __default = concat!($default_str, "\0").as_bytes().as_ptr()
            as *const ::std::ffi::c_char; )?

        #[allow(unused_mut)]
        let mut __hash_vdf_name: *const ::std::ffi::c_char = ::std::ptr::null();
        $( let _ = stringify!($hash_fn);
           __hash_vdf_name = concat!($type_name, "::hash\0").as_bytes().as_ptr()
            as *const ::std::ffi::c_char; )?

        #[allow(unused_mut)]
        let mut __intrinsic_default_vdf_name: *const ::std::ffi::c_char = ::std::ptr::null();
        $( let _ = stringify!($default_fn);
           __intrinsic_default_vdf_name = concat!($type_name, "::intrinsic_default\0").as_bytes().as_ptr()
            as *const ::std::ffi::c_char; )?

        $crate::TypeWithFuncs {
            descriptor: $crate::TypeDescriptor {
                sql_name: concat!($type_name, "\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                persisted_length: -1,
                max_decode_buffer_length: $max_dec,
                encode_vdf_name: concat!($type_name, "::from_string\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                decode_vdf_name: concat!($type_name, "::to_string\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                compare_vdf_name: concat!($type_name, "::compare\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                hash_vdf_name: __hash_vdf_name,
                int_to_params_vdf_name: concat!($type_name, "::int_to_params\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                resolve_params_vdf_name: concat!($type_name, "::resolve_params\0").as_bytes().as_ptr()
                    as *const ::std::ffi::c_char,
                intrinsic_default_str: __default,
                intrinsic_default_vdf_name: __intrinsic_default_vdf_name,
                max_persisted_length: $max_plen,
            },
            embedded_funcs: __embedded,
        }
    }};
}

/// Rust unit testing:
#[cfg(test)]
mod tests {
    use super::*;

    // -- Params::parse: turns "k=v,k=v" into looked-up pairs ---

    #[test]
    fn params_parse_empty() {
        // An empty params string means "no parameters".
        let p = Params::parse("");
        assert!(p.is_empty());
        assert_eq!(p.get("anything"), None);
    }

    #[test]
    fn params_parse_single() {
        let p = Params::parse("width=5");
        assert_eq!(p.len(), 1);
        assert_eq!(p.get("width"), Some("5"));
        assert_eq!(p.get("missing"), None);
    }

    #[test]
    fn params_parse_multiple_preserves_order() {
        let p = Params::parse("dimension=3,type=float");
        assert_eq!(p.get("dimension"), Some("3"));
        assert_eq!(p.get("type"), Some("float"));
        // iter() yields pairs in the order they appeared.
        assert_eq!(
            p.iter().collect::<Vec<_>>(),
            vec![("dimension", "3"), ("type", "float")]
        );
    }

    #[test]
    fn params_parse_entry_without_equals_has_empty_value() {
        // Matches C++ SDK: a chunk that has no '='  -> key with "" value.
        let p = Params::parse("a=1,bad,b=2");
        assert_eq!(p.len(), 3);
        assert_eq!(p.get("a"), Some("1"));
        assert_eq!(p.get("bad"), Some(""));
        assert_eq!(p.get("b"), Some("2"));
    }

    // -- write_inferred_params: serialize pairs into the server's buffer ---

    #[test]
    fn write_inferred_params_basic() {
        let mut buf = [0u8; 64];
        let pairs = vec![
            ("dimension".to_string(), "3".to_string()),
            ("type".to_string(), "float".to_string()),
        ];
        let (needed, _) = write_inferred_params(&mut buf, &pairs);
        assert_eq!(&buf[..needed], b"dimension=3,type=float");
    }

    #[test]
    fn write_inferred_params_sorts_keys() {
        // Given out of order, output is sorted by key (canonical form).
        let mut buf = [0u8; 64];
        let pairs = vec![
            ("type".to_string(), "float".to_string()),
            ("dimension".to_string(), "3".to_string()),
        ];
        let (needed, _) = write_inferred_params(&mut buf, &pairs);
        assert_eq!(&buf[..needed], b"dimension=3,type=float");
    }

    #[test]
    fn write_inferred_params_overflow_reports_full_length() {
        // Buffer too small: still report the FULL length needed (snprintf-style),
        // and flag overflow so the server can retry with a bigger buffer.
        let mut buf = [0u8; 5];
        let pairs = vec![
            ("dimension".to_string(), "3".to_string()),
            ("type".to_string(), "float".to_string()),
        ];
        let (needed, overflow) = write_inferred_params(&mut buf, &pairs);
        assert!(overflow);
        assert_eq!(needed, 22); // "dimension=3,type=float" is 22 bytes
    }

    #[test]
    fn write_inferred_params_empty() {
        let mut buf = [0u8; 8];
        let (needed, overflow) = write_inferred_params(&mut buf, &[]);
        assert_eq!(needed, 0);
        assert!(!overflow);
    }
}
