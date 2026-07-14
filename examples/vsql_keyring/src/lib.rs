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
/// Writes `value` to the keyring under `data_id`. Returns 0 on success, or NULL
/// if `data_id`/`value` is missing or the write fails (capability unavailable,
/// no keyring component, or another error).
fn keyring_store_impl(args: &[InValue]) -> VdfReturn {
    let Some(data_id) = arg_cstr(args.first()) else {
        return VdfReturn::null();
    };
    let auth_id = arg_cstr(args.get(1));
    let Some(&InValue::String(value)) = args.get(2) else {
        return VdfReturn::null();
    };

    match KEYRING.write(&data_id, auth_id.as_deref(), value.as_bytes()) {
        Ok(()) => VdfReturn::int(0),
        Err(_) => VdfReturn::null(),
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

    let mut buf = [0u8; MAX_SECRET_LEN];
    match KEYRING.read(&data_id, auth_id.as_deref(), &mut buf) {
        Ok(Some(n)) => match std::str::from_utf8(&buf[..n]) {
            Ok(s) => VdfReturn::string(s),
            Err(_) => VdfReturn::null(),
        },
        Ok(None) | Err(_) => VdfReturn::null(),
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
