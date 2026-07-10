//! Example VillageSQL extension exercising the `vsql::preview::thread_worker`
//! capability. A background worker ticks a counter every 100ms while its control
//! sys var is ON; `vsql_thread_worker.ticks()` reads the counter.

use std::sync::atomic::{AtomicI64, Ordering};

use villagesql::preview::thread_worker::{
    NextWakeup, ThreadHandle, ThreadWorkerCapability, WakeupReason,
};
use villagesql::{InValue, VdfReturn};

/// Bumped by the background worker on each periodic wakeup.
static TICKS: AtomicI64 = AtomicI64::new(0);

fn worker(reason: WakeupReason, _handle: ThreadHandle) -> NextWakeup {
    if reason == WakeupReason::Periodic {
        TICKS.fetch_add(1, Ordering::Relaxed);
    }
    // Keep the 100ms interval from new(); nothing to do on enable/disable/poll.
    NextWakeup::unchanged()
}

/// Run `worker`, control-var suffix "ticker", tick every 100ms, default var name.
static WORKER: ThreadWorkerCapability = ThreadWorkerCapability::new(worker, c"ticker", 100, None);

/// SQL: `vsql_thread_worker.ticks()` -> INT - how many times the worker has ticked.
fn ticks_impl(_args: &[InValue]) -> VdfReturn {
    VdfReturn::int(TICKS.load(Ordering::Relaxed))
}

villagesql::extension! {
    funcs: [
        villagesql::func!(ticks_impl, "ticks", [] -> villagesql::Type::Int),
    ],
    requires: [
        &WORKER,
    ]
}
