//! Example `VillageSQL` extension exercising the `vsql::sys_var` capability.
//!
//! Declares three system variables — one of each server-supported type — plus an
//! `on_change` callback on the bool that counts how many times it has been set,
//! exposed via `vsql_sys_var.change_count()`:
//!
//! * `enabled`   BOOL   (default true)   — has an `on_change` callback
//! * `threshold` INT    (default 1000, range 0..60000)
//! * `log_path`  STRING (default "`/tmp/vsql_sys_var.log`")
//!
//! ```sql
//! SELECT @@global.vsql_sys_var.enabled;     -- 1
//! SET GLOBAL vsql_sys_var.enabled = 0;
//! SELECT vsql_sys_var.change_count();       -- 1  (callback fired)
//! SELECT @@global.vsql_sys_var.threshold;   -- 1000
//! SELECT @@global.vsql_sys_var.log_path;    -- /tmp/vsql_sys_var.log
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use std::ffi::{c_char, c_void, CStr};
use villagesql::preview::sys_var::{SysVarCapability, SysVarSpec};
use villagesql::sys::vef_sys_var_change_t;
use villagesql::{InValue, VdfReturn};

/// Counts how many times `enabled` has changed. Bumped by the `on_change` callback.
static CHANGE_COUNT: AtomicU64 = AtomicU64::new(0);

// The C allocator's free() — releases the string get() writes into *val.
extern "C" {
    fn free(ptr: *mut c_void);
}

/// Called by the server after `enabled` is set. Runs on the server's thread, so
/// it must be quick and must not panic — a single atomic increment is both.
unsafe extern "C" fn on_enabled_change(_change: *const vef_sys_var_change_t) {
    CHANGE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// The system variables this extension declares.
static SPECS: &[SysVarSpec] = &[
    SysVarSpec::Bool {
        name: c"enabled",
        comment: c"Enable the feature",
        default: true,
        on_change: Some(on_enabled_change),
    },
    SysVarSpec::Int {
        name: c"threshold",
        comment: c"Threshold in milliseconds",
        default: 1000,
        min: 0,
        max: 60000,
        on_change: None,
    },
    SysVarSpec::Str {
        name: c"log_path",
        comment: c"Path to the log file",
        default: c"/tmp/vsql_sys_var.log",
        on_change: None,
    },
];

/// The `sys_var` capability instance, declared `static` so it lives for the whole
/// time the extension is loaded; the server populates it at registration.
static SYS_VAR: SysVarCapability = SysVarCapability::new(SPECS);

/// SQL: `vsql_sys_var.change_count()` -> INT
// The change counter never approaches i64::MAX, so this cast cannot wrap.
#[allow(clippy::cast_possible_wrap)]
fn change_count_impl(_args: &[InValue]) -> VdfReturn {
    VdfReturn::int(CHANGE_COUNT.load(Ordering::Relaxed) as i64)
}

/// SQL: `vsql_sys_var.read_threshold()` -> STRING
///
/// Reads `threshold` back through the Rust `get` accessor (not SQL), exercising
/// the `get()` FFI path end to end.
fn read_threshold_impl(_args: &[InValue]) -> VdfReturn {
    let mut val: *mut c_void = std::ptr::null_mut();
    let mut val_len: usize = 0;
    // SAFETY: names are valid NUL-terminated C strings; val/val_len are valid,
    // writable, and live for the call.
    let result = unsafe {
        SYS_VAR.get(
            c"vsql_sys_var".as_ptr(),
            c"threshold".as_ptr(),
            &raw mut val,
            &raw mut val_len,
        )
    };
    match result {
        // Some(false) = success: *val is a malloc'd NUL-terminated string we now own.
        Some(false) => {
            // SAFETY: on success get() wrote avalid NUL-terminated C string to *val.
            let s = unsafe { CStr::from_ptr(val.cast::<c_char>()) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: *val came from the server'sC malloc; release it per the contract.
            unsafe { free(val) };
            VdfReturn::string(s)
        }
        _ => VdfReturn::null(),
    }
}

/// SQL: `vsql_sys_var.set_threshold_500()` -> INT
///
/// Sets `threshold` to 500 through the Rust `set` accessor, exercising the `set()`
/// FFI path. Returns 1 on success, 0 otherwise.
fn set_threshold_500_impl(_args: &[InValue]) -> VdfReturn {
    // SAFETY: all pointers are validNUL-terminated C strings (scope is null =
    // running value only), live for the call.
    let result = unsafe {
        SYS_VAR.set(
            c"vsql_sys_var".as_ptr(),
            c"threshold".as_ptr(),
            std::ptr::null(),
            c"500".as_ptr(),
        )
    };
    // set() returns Some(false) on success (inverted C bool).
    VdfReturn::int(i64::from(result == Some(false)))
}

villagesql::extension! {
    funcs: [
        villagesql::func!(change_count_impl, "change_count", [] -> villagesql::Type::Int),
        villagesql::func!(read_threshold_impl, "read_threshold", [] -> villagesql::Type::String),
        villagesql::func!(set_threshold_500_impl, "set_threshold_500", [] -> villagesql::Type::Int),
    ],
    requires: [
        &SYS_VAR,
    ]
}
