//! Proof example for the author-supplied postrun hook.
//!
//! `tracked` allocates state in prerun, counts rows, then runs an author
//! postrun before the automatic drop. Every lifecycle step appends to a
//! process-global log. `lifecycle_events` reads it back so an MTR test can
//! assert the exact order: prerun -> postrun -> drop.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use villagesql::{InValue, PrerunArgs, PrerunResult, VdfReturn};

/// Ordered log of lifecycle events, readable from SQL via `lifecycle_events`.
static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Row count the author postrun observed (proves it had live state access).
static ROWS_SEEN_IN_POSTRUN: AtomicI64 = AtomicI64::new(-1);

fn log_event(e: &'static str) {
    EVENTS.lock().unwrap().push(e);
}

/// Per-statement state. Its `Drop` logs "drop", so the test can prove the
/// author postrun runs before the automatic drop
struct Tracked {
    rows: i64,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        log_event("drop");
    }
}

/// prerun: allocate the state, record "prerun".
fn tracked_prerun(_args: PrerunArgs, out: PrerunResult<Tracked>) {
    log_event("prerun");
    out.set_state(Tracked { rows: 0 });
}

/// row: count this row, return the running count.
fn tracked(state: &mut Tracked, _args: &[InValue]) -> VdfReturn {
    state.rows += 1;
    VdfReturn::int(state.rows)
}

/// author postrun: runs after the last row, before the drop. The state is
/// still alive here, so we can read what the rows accumulated.
fn tracked_postrun(state: &mut Tracked) {
    ROWS_SEEN_IN_POSTRUN.store(state.rows, Ordering::Relaxed);
    log_event("postrun");
}

/// Zero-arg reader: return the recorded lifecycle order plus the row count the
/// postrun saw, then reset both so each test run starts clean.
fn lifecycle_events(_args: &[InValue]) -> VdfReturn {
    let mut ev = EVENTS.lock().unwrap();
    let order = ev.join(",");
    ev.clear();
    let rows = ROWS_SEEN_IN_POSTRUN.swap(-1, Ordering::Relaxed);
    VdfReturn::string(format!("{order}|rows_seen={rows}"))
}

villagesql::extension! {
    funcs: [
        villagesql::func!(tracked, "tracked", [] -> villagesql::Type::Int,
            state: Tracked, prerun: tracked_prerun, postrun: tracked_postrun),
        villagesql::func!(lifecycle_events, "lifecycle_events", [] ->
            villagesql::Type::String),
    ]
}
