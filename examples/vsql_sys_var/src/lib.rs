//! Example VillageSQL extension exercising the `vsql::sys_var` capability.
//!
//! Declares three system variables — one of each server-supported type — plus an
//! `on_change` callback on the bool that counts how many times it has been set,
//! exposed via `vsql_sys_var.change_count()`:
//!
//! * `enabled`   BOOL   (default true)   — has an on_change callback
//! * `threshold` INT    (default 1000, range 0..60000)
//! * `log_path`  STRING (default "/tmp/vsql_sys_var.log")
//!
//! ```sql
//! SELECT @@global.vsql_sys_var.enabled;     -- 1
//! SET GLOBAL vsql_sys_var.enabled = 0;
//! SELECT vsql_sys_var.change_count();       -- 1  (callback fired)
//! SELECT @@global.vsql_sys_var.threshold;   -- 1000
//! SELECT @@global.vsql_sys_var.log_path;    -- /tmp/vsql_sys_var.log
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use villagesql::preview::sys_var::{vef_sys_var_change_t, SysVarCapability, SysVarSpec};
use villagesql::{InValue, VdfReturn};

/// Counts how many times `enabled` has changed. Bumped by the on_change callback.
static CHANGE_COUNT: AtomicU64 = AtomicU64::new(0);

/// Called by the server after `enabled` is set. Runs on the server's thread, so
/// it must be quick and must not panic — a single atomic increment is both.
unsafe extern "C" fn on_enabled_change(_change: *const vef_sys_var_change_t) {
    CHANGE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// SQL: vsql_sys_var.change_count() -> INT
fn change_count_impl(_args: &[InValue]) -> VdfReturn {
    VdfReturn::int(CHANGE_COUNT.load(Ordering::Relaxed) as i64)
}

villagesql::extension! {
    funcs: [
        villagesql::func!(change_count_impl, "change_count", [] -> villagesql::Type::Int),
    ],
    capabilities: [
        SysVarCapability::request(&[
            SysVarSpec::Bool {
                name: b"enabled\0",
                comment: b"Enable the feature\0",
                default: true,
                on_change: Some(on_enabled_change),
            },
            SysVarSpec::Int {
                name: b"threshold\0",
                comment: b"Threshold in milliseconds\0",
                default: 1000,
                min: 0,
                max: 60000,
                on_change: None,
            },
            SysVarSpec::Str {
                name: b"log_path\0",
                comment: b"Path to the log file\0",
                default: b"/tmp/vsql_sys_var.log\0",
                on_change: None,
            },
        ]),
    ]
}
