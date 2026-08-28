//! Example `VillageSQL` extension exercising `vsql::preview::sql_query` end to end.
//! A background worker opens a SQL session on its own thread and runs a suite of
//! queries covering the whole API, including buffered `execute` + row iteration,
//! int/str/real columns, SQL NULL, streaming `for_each` over multiple rows, and
//! error diagnostics. Each passing sub-check sets a bit. `probe_checks()` reads
//! the mask. Proves in-process SQL from a worker thread.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use villagesql::preview::sql_query::SqlQueryCapability;
use villagesql::preview::thread_worker::{
    NextWakeup, ThreadHandle, ThreadWorkerCapability, WakeupReason,
};
use villagesql::{InValue, VdfReturn};

/// Bitmask of passed sub-checks. `-1` until the worker first runs. `-2` if it
/// couldn't open a session (capability off).
static CHECKS: AtomicI64 = AtomicI64::new(-1);

const CHECK_INT: i64 = 1 << 0; // execute + next_row + get_int
const CHECK_STR: i64 = 1 << 1; // get_str
const CHECK_NULL: i64 = 1 << 2; // Option::None for SQL NULL
const CHECK_REAL: i64 = 1 << 3; // get_real
const CHECK_STREAM: i64 = 1 << 4; // for_each over multiple rows
const CHECK_ERROR: i64 = 1 << 5; // has_error + error() diagnostics

/// Run `worker`, control-var suffix "probe", every 100ms.
static WORKER: ThreadWorkerCapability =
    ThreadWorkerCapability::new(worker, "probe", Duration::from_millis(100), None);

/// The `sql_query` capability. The worker opens sessions through it.
static SQL_QUERY: SqlQueryCapability = SqlQueryCapability::new();

fn worker(reason: WakeupReason, handle: &ThreadHandle) -> NextWakeup {
    if reason == WakeupReason::Periodic {
        run_probe(handle);
    }
    NextWakeup::unchanged()
}

/// On the worker thread: open a session, run the full check suite, record which
/// API paths behaved correctly as a bitmask (`63` = 111111 = all six).
fn run_probe(handle: &ThreadHandle) {
    let Some(session) = SQL_QUERY.open(handle) else {
        CHECKS.store(-2, Ordering::Relaxed);
        return;
    };
    let mut mask: i64 = 0;

    // Buffered execute + typed int column.
    if let Some(mut r) = session.execute("SELECT 1 + 1") {
        if !r.has_error() && r.next_row().is_some_and(|row| row.get_int(0) == Some(2)) {
            mask |= CHECK_INT;
        }
    }
    // String column.
    if let Some(mut r) = session.execute("SELECT 'hi'") {
        if r.next_row().is_some_and(|row| row.get_str(0) == Some("hi")) {
            mask |= CHECK_STR;
        }
    }
    // SQL NULL -> None.
    if let Some(mut r) = session.execute("SELECT NULL") {
        if r.next_row().is_some_and(|row| row.get_str(0).is_none()) {
            mask |= CHECK_NULL;
        }
    }
    // Float column.
    if let Some(mut r) = session.execute("SELECT 1.5") {
        if r.next_row()
            .is_some_and(|row| row.get_real(0).is_some_and(|v| (v - 1.5).abs() < 1e-9))
        {
            mask |= CHECK_REAL;
        }
    }
    // Streaming for_each over multiple rows.
    let mut sum = 0i64;
    if let Some(r) = session.for_each("SELECT 1 UNION SELECT 2 UNION SELECT 3", |row| {
        if let Some(v) = row.get_int(0) {
            sum += v;
        }
    }) {
        if !r.has_error() && sum == 6 {
            mask |= CHECK_STREAM;
        }
    }
    // Error diagnostics on a bad statement.
    if let Some(r) = session.execute("SELECT * FROM vsql_sql_query_no_such_table") {
        if r.has_error() && r.error().is_some() {
            mask |= CHECK_ERROR;
        }
    }

    CHECKS.store(mask, Ordering::Relaxed);
}

/// SQL: `vsql_sql_query.probe_checks()` -> INT - bitmask of passed sub-checks
/// (`63` = all six; `-1` before the worker runs; `-2` if it can't open a session).
fn probe_checks_impl(_args: &[InValue]) -> VdfReturn {
    VdfReturn::int(CHECKS.load(Ordering::Relaxed))
}

villagesql::extension! {
    funcs: [
        villagesql::func!(probe_checks_impl, "probe_checks", [] -> villagesql::Type::Int),
    ],
    requires: [
        &WORKER,
        &SQL_QUERY,
    ]
}
