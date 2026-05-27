//! Safe Rust SDK for writing VillageSQL extension functions (VDFs) and custom types.
//!
//! # Quick start (functions)
//!
//! 1. Add `vsql = { path = "..." }` to your `Cargo.toml` with `crate-type = ["cdylib"]`.
//! 2. Write a function with the signature `fn(&[InValue]) -> VdfReturn`.
//! 3. Declare the extension with the [`extension!`] macro.
//!
//! See `examples/vsql_rot13` for a complete working function extension.
//! See `examples/vsql_rational` for a complete working custom type extension.

pub use paste;
pub use vsql_sys as sys;

use std::ffi::c_char;
use vsql_sys::{
    vef_func_desc_t, vef_protocol_t_VEF_PROTOCOL_3, vef_registration_t,
    vef_return_value_type_t_VEF_RESULT_ERROR, vef_return_value_type_t_VEF_RESULT_NULL,
    vef_return_value_type_t_VEF_RESULT_VALUE, vef_return_value_type_t_VEF_RESULT_WARNING,
    vef_signature_t, vef_type_desc_t, vef_type_id_VEF_TYPE_CUSTOM, vef_type_id_VEF_TYPE_INT,
    vef_type_id_VEF_TYPE_REAL, vef_type_id_VEF_TYPE_STRING, vef_type_t, vef_vdf_args_t,
    vef_vdf_result_t, vef_version_t, VEF_MAX_ERROR_LEN,
};

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
/// For custom types the binary persisted bytes are exposed as [`InValue::Custom`].
#[derive(Debug)]
pub enum InValue<'a> {
    Null,
    String(&'a str),
    Real(f64),
    Int(i64),
    /// Raw binary bytes for a custom-type argument (persisted format).
    Custom(&'a [u8]),
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
    pub fn null() -> Self {
        Self::Null
    }
    pub fn string(s: impl Into<std::string::String>) -> Self {
        Self::String(s.into())
    }
    pub fn real(v: f64) -> Self {
        Self::Real(v)
    }
    pub fn int(v: i64) -> Self {
        Self::Int(v)
    }
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
        *mut vsql_sys::vef_context_t,
        *mut vef_vdf_args_t,
        *mut vef_vdf_result_t,
    ),
    pub deterministic: bool,
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
    /// Optional null-terminated default value string (encoded at install time).
    pub intrinsic_default_str: *const c_char,
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

// ── Internal runtime helpers ──────────────────────────────────────────────────

/// Convert raw Protocol-3 VDF arguments into a `&[InValue]` slice and call `f`.
///
/// # Safety
/// `args` and `result` must be valid for the duration of the call.
pub unsafe fn dispatch_vdf(
    f: fn(&[InValue]) -> VdfReturn,
    args: *mut vef_vdf_args_t,
    result: *mut vef_vdf_result_t,
) {
    let args = &*args;
    let result = &mut *result;

    // Protocol 3: values is an array of *mut vef_invalue_t pointers.
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
                let bytes = std::slice::from_raw_parts(anon.str_value as *const u8, anon.str_len);
                InValue::String(std::str::from_utf8_unchecked(bytes))
            }
            t if t == vef_type_id_VEF_TYPE_REAL => InValue::Real(v.__bindgen_anon_1.real_value),
            t if t == vef_type_id_VEF_TYPE_INT => InValue::Int(v.__bindgen_anon_1.int_value),
            t if t == vef_type_id_VEF_TYPE_CUSTOM => {
                let anon = &v.__bindgen_anon_1.__bindgen_anon_2;
                let bytes = std::slice::from_raw_parts(anon.bin_value, anon.bin_len);
                InValue::Custom(bytes)
            }
            _ => InValue::Null,
        };
        in_values.push(iv);
    }

    write_result(f(&in_values), result);
}

unsafe fn write_result(ret: VdfReturn, result: &mut vef_vdf_result_t) {
    match ret {
        VdfReturn::Null => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_NULL;
        }
        VdfReturn::String(s) => {
            result.type_ = vef_return_value_type_t_VEF_RESULT_VALUE;
            let anon = &mut result.__bindgen_anon_1.__bindgen_anon_1;
            let bytes = s.as_bytes();
            let n = bytes.len().min(anon.max_str_len);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), anon.str_buf as *mut u8, n);
            result.actual_len = n;
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
            result.type_ = vef_return_value_type_t_VEF_RESULT_VALUE;
            let anon = &mut result.__bindgen_anon_1.__bindgen_anon_2;
            let n = bytes.len().min(anon.max_bin_len);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), anon.bin_buf, n);
            result.actual_len = n;
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

unsafe fn write_error_msg(msg: &[u8], buf: *mut c_char) {
    let max = (VEF_MAX_ERROR_LEN as usize).saturating_sub(1);
    let n = msg.len().min(max);
    std::ptr::copy_nonoverlapping(msg.as_ptr(), buf as *mut u8, n);
    *buf.add(n) = 0;
}

unsafe fn build_func_ptr(d: &FuncDescriptor) -> *mut vef_func_desc_t {
    let params: Box<[vef_type_t]> = d.params.iter().map(|t| t.to_raw()).collect();
    let param_count = params.len() as u32;
    let params_ptr = Box::into_raw(params) as *const vef_type_t;
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
        prerun: None,
        postrun: None,
        buffer_size: 0,
        deterministic: d.deterministic,
        clear: None,
        accumulate: None,
    }))
}

/// Allocate a `vef_registration_t` from slices of descriptors.
///
/// # Safety
/// All descriptor fields must be valid for `'static`.
pub unsafe fn build_registration(
    funcs: &[FuncDescriptor],
    types: &[TypeWithFuncs],
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
    let func_count = func_ptrs.len() as u32;
    let funcs_ptr = Box::into_raw(func_ptrs.into_boxed_slice()) as *mut *mut vef_func_desc_t;

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
            int_to_params_vdf_name: std::ptr::null(),
            resolve_params_vdf_name: std::ptr::null(),
            intrinsic_default_vdf_name: std::ptr::null(),
            intrinsic_default_str: t.descriptor.intrinsic_default_str,
            max_persisted_length: 0,
        })));
    }
    let type_count = type_ptrs.len() as u32;
    let types_ptr = Box::into_raw(type_ptrs.into_boxed_slice()) as *mut *mut vef_type_desc_t;

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
        required_capability_count: 0,
        required_capabilities: std::ptr::null(),
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
            sig.params as *mut vef_type_t,
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

    drop(reg);
}

// ── Macros ────────────────────────────────────────────────────────────────────

/// Produce a [`Type::Custom`] value for the named custom type.
///
/// ```ignore
/// vsql::custom!("rational")   // → Type::Custom pointing to b"rational\0"
/// ```
#[macro_export]
macro_rules! custom {
    ($name:literal) => {
        $crate::Type::Custom(concat!($name, "\0").as_bytes().as_ptr() as *const ::std::ffi::c_char)
    };
}

/// Declare a VillageSQL extension and generate the `vef_register` /
/// `vef_unregister` C entry points.
///
/// ```ignore
/// vsql::extension! {
///     funcs: [
///         vsql::func!(my_impl, "sql_name", [vsql::Type::String] -> vsql::Type::String),
///     ],
///     types: [
///         vsql::custom_type!(
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
    // With types list.
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ],
        types: [ $( $type_desc:expr ),* $(,)? ] $(,)?
    ) => {
        #[no_mangle]
        pub unsafe extern "C" fn vef_register(
            _arg: *const $crate::sys::vef_register_arg_t,
        ) -> *mut $crate::sys::vef_registration_t {
            let funcs: &[$crate::FuncDescriptor] = &[$($func_desc),*];
            let types: ::std::vec::Vec<$crate::TypeWithFuncs> = vec![$($type_desc),*];
            $crate::build_registration(funcs, &types)
        }

        #[no_mangle]
        pub unsafe extern "C" fn vef_unregister(
            _arg: *const $crate::sys::vef_unregister_arg_t,
            registration: *mut $crate::sys::vef_registration_t,
        ) {
            $crate::free_registration(registration);
        }
    };
    // Without types list (backward compatible).
    (
        funcs: [ $( $func_desc:expr ),* $(,)? ] $(,)?
    ) => {
        $crate::extension! {
            funcs: [ $($func_desc),* ],
            types: []
        }
    };
}

/// Build a [`FuncDescriptor`] for one VDF and generate its `extern "C"` trampoline.
///
/// ```ignore
/// vsql::func!(impl_fn, "sql_name", [vsql::Type::String] -> vsql::Type::String)
/// vsql::func!(impl_fn, "sql_name", [vsql::custom!("my_type")] -> vsql::custom!("my_type"),
///             deterministic: true)
/// ```
#[macro_export]
macro_rules! func {
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr,
     deterministic: $det:expr) => {{
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
                deterministic: $det,
            }
        }
    }};
    ($impl_fn:ident, $sql_name:literal, [$($param:expr),* $(,)?] -> $ret:expr) => {
        $crate::func!($impl_fn, $sql_name, [$($param),*] -> $ret, deterministic: false)
    };
}

/// Build a [`TypeWithFuncs`] for a custom SQL type, generating the four
/// SQL-callable VDFs (`TYPE::from_string`, `TYPE::to_string`, `TYPE::compare`,
/// and optionally `TYPE::hash`) automatically.
///
/// Required fields: `type_name`, `persisted_length`, `max_decode_buffer_length`,
/// `encode`, `decode`, `compare`.
///
/// Optional fields: `hash` (recommended for indexed columns),
/// `default` (string literal encoded at install time).
///
/// Rust function signatures required:
/// - `encode`: `fn(&str) -> Result<Vec<u8>, String>`
/// - `decode`: `fn(&[u8]) -> Result<String, String>`
/// - `compare`: `fn(&[u8], &[u8]) -> std::cmp::Ordering`
/// - `hash`: `fn(&[u8]) -> usize`
///
/// ```ignore
/// vsql::custom_type!(
///     type_name: "rational",
///     persisted_length: 16,
///     max_decode_buffer_length: 42,
///     encode: rational_encode,
///     decode: rational_decode,
///     compare: rational_compare,
///     hash: rational_hash,
///     default: "0/1",
/// )
/// ```
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
        $crate::paste::paste! {
            // ── TYPE::from_string(STRING) -> CUSTOM ───────────────────────────
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

            // ── TYPE::to_string(CUSTOM) -> STRING ─────────────────────────────
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

            // ── TYPE::compare(CUSTOM, CUSTOM) -> INT ──────────────────────────
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

            // ── TYPE::hash(CUSTOM) -> INT (optional) ──────────────────────────
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

            // ── Assemble TypeWithFuncs ─────────────────────────────────────────
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
            let mut __embedded: ::std::vec::Vec<$crate::FuncDescriptor> =
                ::std::vec::Vec::new();

            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::from_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_FROM_STRING_PARAMS_ $enc_fn:upper >],
                returns: $crate::custom!($type_name),
                trampoline: [< __vsql_trampoline_from_string_ $enc_fn >],
                deterministic: true,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::to_string\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_TO_STRING_PARAMS_ $dec_fn:upper >],
                returns: $crate::Type::String,
                trampoline: [< __vsql_trampoline_to_string_ $dec_fn >],
                deterministic: true,
            });
            __embedded.push($crate::FuncDescriptor {
                sql_name: concat!($type_name, "::compare\0").as_bytes().as_ptr()
                    as *const ::std::os::raw::c_char,
                params: [< __VSQL_COMPARE_PARAMS_ $cmp_fn:upper >],
                returns: $crate::Type::Int,
                trampoline: [< __vsql_trampoline_compare_ $cmp_fn >],
                deterministic: true,
            });
            $(
                __embedded.push($crate::FuncDescriptor {
                    sql_name: concat!($type_name, "::hash\0").as_bytes().as_ptr()
                        as *const ::std::os::raw::c_char,
                    params: [< __VSQL_HASH_PARAMS_ $hash_fn:upper >],
                    returns: $crate::Type::Int,
                    trampoline: [< __vsql_trampoline_hash_ $hash_fn >],
                    deterministic: true,
                });
            )?

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
                    intrinsic_default_str: __default,
                },
                embedded_funcs: __embedded,
            }
        }
    }};
}
