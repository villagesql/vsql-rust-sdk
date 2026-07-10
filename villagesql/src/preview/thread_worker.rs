//! Idiomatic wrapper for the `vsql::preview::thread_worker` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/thread_worker.h`.
//! This is a preview capability; its ABI may change or be removed in future versions.

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicPtr;

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_next_wakeup_t, vef_preview_thread_worker_t, vef_thread_handle_t,
    vef_thread_worker_descriptor_t, vef_wakeup_reason_t, VEF_PREVIEW_THREAD_WORKER_NAME,
};
// bindgen prefixes the enum constants so we alias them here to readable names.
use crate::sys::{
    vef_wakeup_reason_t_VEF_WAKEUP_DISABLE as VEF_WAKEUP_DISABLE,
    vef_wakeup_reason_t_VEF_WAKEUP_ENABLE as VEF_WAKEUP_ENABLE,
    vef_wakeup_reason_t_VEF_WAKEUP_PERIODIC as VEF_WAKEUP_PERIODIC,
    vef_wakeup_reason_t_VEF_WAKEUP_POLL_FD as VEF_WAKEUP_POLL_FD,
};

const VTABLE_HASH: &[u8] = b"ver-1\0";
const CONFIG_HASH: &[u8] = b"ver-1\0";

/// Why the server woke up your worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeupReason {
    Enable,
    Periodic,
    PollFd,
    Disable,
}

/// What your worker returns: when to wake next, and which fd to watch.
#[derive(Debug, Clone, Copy)]
pub struct NextWakeup {
    /// Milliseconds to next periodic wakeup. `0` = keep current interval.
    pub sleep_ms: u32,
    /// FD to watch: `> 0` watch, `-1` clear, `0` = keep current.
    pub poll_fd: i32,
}

impl NextWakeup {
    /// Keep interval and poll fd unchanged.
    #[must_use]
    pub const fn unchanged() -> Self {
        Self {
            sleep_ms: 0,
            poll_fd: 0,
        }
    }
    /// Wake again after `ms` milliseconds (poll fd unchanged).
    #[must_use]
    pub const fn after_ms(ms: u32) -> Self {
        Self {
            sleep_ms: ms,
            poll_fd: 0,
        }
    }
}

impl From<NextWakeup> for vef_next_wakeup_t {
    fn from(n: NextWakeup) -> Self {
        Self {
            sleep_ms: n.sleep_ms,
            poll_fd: n.poll_fd,
        }
    }
}

/// Opaque handle passed to the work function. Reserved for calling `sql_query`
/// from the worker thread once that capability is ported - no methods yet.
#[allow(dead_code)] // the inner pointer isn't read until sql_query lands
pub struct ThreadHandle(*mut vef_thread_handle_t);

/// The work function you write. Runs on the server's worker thread.
pub type WorkFn = fn(WakeupReason, ThreadHandle) -> NextWakeup;

/// The `vsql::preview::thread_worker` capability. Declare it as a `static` and
/// list it via `requires: [&WORKER]`.
pub struct ThreadWorkerCapability {
    abi_: AtomicPtr<vef_preview_thread_worker_t>,
    work_fn: WorkFn,
    sleep_ms: u32,
    suffix: &'static CStr,
    var_name: Option<&'static CStr>,
}

const _: () = {
    const fn assert_sync<T: Sync>() {}
    assert_sync::<ThreadWorkerCapability>();
};

impl ThreadWorkerCapability {
    /// Create a thread-worker capability. `suffix` names the worker/control var
    /// (`{suffix}_enabled`); pass `var_name: Some(..)` to override that name.
    #[must_use]
    pub const fn new(
        work_fn: WorkFn,
        suffix: &'static CStr,
        sleep_ms: u32,
        var_name: Option<&'static CStr>,
    ) -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
            work_fn,
            sleep_ms,
            suffix,
            var_name,
        }
    }
}

impl Capability for &'static ThreadWorkerCapability {
    fn request(self) -> RequiredCapability {
        // Leaked with Box::into_raw: the server keeps the descriptor
        // (capability_config) for the extension's lifetime.
        let descriptor = vef_thread_worker_descriptor_t {
            // Point the server at our trampoline, not the user's fn directly.
            work_fn: Some(trampoline),
            // Pointer back to this 'static capability, which the trampoline
            // recovers to reach work_fn.
            arg: std::ptr::from_ref(self).cast::<c_void>().cast_mut(),
            sleep_ms: self.sleep_ms,
            suffix: self.suffix.as_ptr(),
            // Option<&CStr> -> pointer, or null when None (server uses the default name).
            var_name: self.var_name.map_or(std::ptr::null(), CStr::as_ptr),
        };
        let descriptor_ptr: *const vef_thread_worker_descriptor_t =
            Box::into_raw(Box::new(descriptor));

        RequiredCapability {
            name: VEF_PREVIEW_THREAD_WORKER_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: CONFIG_HASH.as_ptr().cast::<c_char>(),
            capability_config: descriptor_ptr.cast::<c_void>(),
        }
    }
}

/// The C entry point the server actually calls. Recovers our capability from
/// `arg`, translates the C-shaped call into a safe Rust call to the user's work
/// function, and translates the result back.
///
/// # Safety
/// The server only ever calls this with the `arg` we set in `request()` - a
/// pointer to the `'static ThreadWorkerCapability`.
unsafe extern "C" fn trampoline(
    reason: vef_wakeup_reason_t,
    thread: *mut vef_thread_handle_t,
    arg: *mut c_void,
) -> vef_next_wakeup_t {
    // SAFETY: request() set arg = the address of a 'static ThreadWorkerCapability.
    let cap = unsafe { &*(arg.cast::<ThreadWorkerCapability>()) };

    let reason = match reason {
        VEF_WAKEUP_ENABLE => WakeupReason::Enable,
        VEF_WAKEUP_PERIODIC => WakeupReason::Periodic,
        VEF_WAKEUP_POLL_FD => WakeupReason::PollFd,
        VEF_WAKEUP_DISABLE => WakeupReason::Disable,
        _ => return NextWakeup::unchanged().into(), // unknown -> no change
    };

    // Run the user's safe Rust function.
    let next = catch_unwind(AssertUnwindSafe(|| {
        (cap.work_fn)(reason, ThreadHandle(thread))
    }))
    .unwrap_or_else(|_| NextWakeup::unchanged());

    next.into()
}
