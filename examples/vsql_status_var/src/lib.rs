//! Example `VillageSQL` extension exercising the `vsql::status_var` capability.
//! Declares two status variables the server exposes via `SHOW STATUS` as
//! `vsql_status_var.<name>`:
//! * `requests` INT - a counter bumed by `vsql_status_var.bump()`
//! * `load` DOUBLE - a gauge (default 0.5)
//!
//! ```sql
//! SELECT vsql_status_var.bump();                      -- 1
//! SHOW GLOBAL STATUS LIKE 'vsql_status_var.%';        -- requests=1, load=0.500000
//! ```

use std::sync::atomic::{AtomicI64, Ordering};

use villagesql::preview::status_var::{AtomicF64, StatusVarCapability, StatusVarSpec};
use villagesql::{InValue, VdfReturn};

/// The counter behind the `requests` status var. Owned here, read by the server.
static REQUESTS: AtomicI64 = AtomicI64::new(0);
/// The gauge behind the `load` status var.
static LOAD: AtomicF64 = AtomicF64::new(0.5);

static SPECS: &[StatusVarSpec] = &[
    StatusVarSpec::Int {
        name: c"requests",
        value: &REQUESTS,
    },
    StatusVarSpec::Double {
        name: c"load",
        value: &LOAD,
    },
];

static STATUS_VAR: StatusVarCapability = StatusVarCapability::new(SPECS);

/// SQL: `vsql_status_var.bump()` -> INT - incrememnts `requests`, returns the new count.
fn bump_impl(_args: &[InValue]) -> VdfReturn {
    let n = REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
    VdfReturn::int(n)
}

villagesql::extension! {
    funcs: [
        villagesql::func!(bump_impl, "bump", [] -> villagesql::Type::Int),
    ],
    requires: [
        &STATUS_VAR,
    ]
}
