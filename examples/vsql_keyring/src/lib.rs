//! Example `VillageSQL` extension exercising the `vsql::preview::keyring` capability.
//!
//! Exposes two SQL functions backed by the Rust keyring accessors:
//! * `keyring_store(data_id, auth_id, value)` — write a secret via `KEYRING.write`
//! * `keyring_read(data_id, auth_id)` — read it back via `KEYRING.read`
//!
//! Requires a `MySQL` keyring component (e.g. `component_keyring_file`) on the server.
//!
//! ```sql
//! SELECT vsql_keyring.keyring_store('my_key', NULL, 'my_secret');  -- 0 (OK)
//! SELECT vsql_keyring.keyring_read('my_key', NULL);                -- my_secret
//! ```

use std::ffi::CString;

use villagesql::preview::keyring::KeyringCapability;
use villagesql::sys::vef_keyring_result_t_VEF_KEYRING_OK as VEF_KEYRING_OK;
use villagesql::{InValue, VdfReturn};

/// The keyring capability instance, declared `static` for the extension's life.
static KEYRING: KeyringCapability = KeyringCapability::new();

/// Largest secret this example can read back. The `MySQL` keyring has no
/// size-probe: `read_keyring` returns `NOT_FOUND` (not the real length) when a
/// secret is bigger than the buffer we hand it, so a fixed cap is the best this
/// ABI allows — a larger secret reads back as SQL NULL. Kept in sync with
/// `keyring_read`'s `buffer_size` so anything that fits the read buffer also
/// fits the SQL result buffer.
const MAX_SECRET_LEN: usize = 1024;

/// A STRING argument as an owned C string. `None` for SQL NULL, a missing arg,
/// a non-string, or a string with an interior NUL (which can't be a C string).
fn arg_cstr(arg: Option<&InValue>) -> Option<CString> {
    match arg {
        Some(&InValue::String(s)) => CString::new(s).ok(),
        _ => None,
    }
}

/// SQL: `vsql_keyring.keyring_store(data_id, auth_id, value)` -> INT
///
/// Writes `value` to the keyring under `data_id`. Returns the keyring result code
/// (0 = `VEF_KEYRING_OK`), or NULL if `data_id`/`value` is missing or the
/// capability is unavailable.
fn keyring_store_impl(args: &[InValue]) -> VdfReturn {
    let Some(data_id) = arg_cstr(args.first()) else {
        return VdfReturn::null();
    };
    let auth_id = arg_cstr(args.get(1));
    let auth_ptr = auth_id.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    let Some(&InValue::String(value)) = args.get(2) else {
        return VdfReturn::null();
    };

    // SAFETY: data_id/auth (or null) are valid NUL-terminated C strings; value
    // points to `value.len()` readable bytes — all valid for the call.
    let result = unsafe { KEYRING.write(data_id.as_ptr(), auth_ptr, value.as_ptr(), value.len()) };
    match result {
        Some(r) => VdfReturn::int(i64::from(r)),
        None => VdfReturn::null(),
    }
}

/// SQL: `vsql_keyring.keyring_read(data_id, auth_id)` -> STRING
///
/// Reads the secret stored under `data_id`. Returns the value on success, or NULL
/// if `data_id` is missing/NULL, the key doesn't exist, the capability is
/// unavailable, or the secret isn't valid UTF-8 text.
fn keyring_read_impl(args: &[InValue]) -> VdfReturn {
    let Some(data_id) = arg_cstr(args.first()) else {
        return VdfReturn::null();
    };
    let auth_id = arg_cstr(args.get(1));
    let auth_ptr = auth_id.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

    let mut buf = [0u8; MAX_SECRET_LEN];
    let mut out_len: usize = 0;
    // SAFETY: data_id/auth (or null) are valid NUL-terminated C strings; buf is a
    // writable 1024-byte buffer and out_len a valid writable pointer — all valid
    // for the call.
    let result = unsafe {
        KEYRING.read(
            data_id.as_ptr(),
            auth_ptr,
            buf.as_mut_ptr(),
            buf.len(),
            &raw mut out_len,
        )
    };
    match result {
        Some(r) if r == VEF_KEYRING_OK => {
            let n = out_len.min(buf.len());
            match std::str::from_utf8(&buf[..n]) {
                Ok(s) => VdfReturn::string(s),
                Err(_) => VdfReturn::null(),
            }
        }
        _ => VdfReturn::null(),
    }
}

villagesql::extension! {
    funcs: [
        villagesql::func!(
            keyring_store_impl, "keyring_store",
            [villagesql::Type::String, villagesql::Type::String, villagesql::Type::String]
                -> villagesql::Type::Int
        ),
        villagesql::func!(
            keyring_read_impl, "keyring_read",
            [villagesql::Type::String, villagesql::Type::String] -> villagesql::Type::String,
            buffer_size: MAX_SECRET_LEN
        ),
    ],
    requires: [
        &KEYRING,
    ]
}
