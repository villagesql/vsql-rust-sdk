//! Example VillageSQL extension exercising the `vsql::sys_var` capability.
//!
//! Declares one boolean system variable, `enabled` (default `true`), with an
//! `on_change` callback that counts how many times it has been set. The count is
//! exposed via `vsql_sys_var.change_count()`, so a test can prove the server
//! actually called back into the extension:
//!
//! ```sql
//! SELECT @@global.vsql_sys_var.enabled;   -- 1 (default)
//! SELECT vsql_sys_var.change_count();     -- 0
//! SET GLOBAL vsql_sys_var.enabled = 0;
//! SELECT vsql_sys_var.change_count();     -- 1  (callback fired)
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
        ]),
    ]
}
