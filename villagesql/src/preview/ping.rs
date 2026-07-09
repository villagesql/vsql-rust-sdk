//! Idiomatic wrapper for the `vsql::preview::ping` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/ping.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{vef_preview_ping_t, VEF_PREVIEW_PING_ABI_VERSION, VEF_PREVIEW_PING_NAME};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against (its `vef_preview_ping_t` entry
// is registered as "ver-1"). NUL-terminated; `strcmp`'d server-side.
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// The `vsql::preview::ping` capability. Declare it in your extension and
/// list it via `requires: [&PING]`; the server populates it at load time,
/// after which [`PingCapability::ping`] returns the counter.
pub struct PingCapability {
    /// The slot the server fills with its `vef_preview_ping_t*` vtable
    /// at load time. `AtomicPtr<T>` is layout-identical to `*mut T`, so
    /// handing its address to the server as `vtable_dest` is safe. The
    /// atomic makes it so the Rust side read it without a data race.
    abi_: AtomicPtr<vef_preview_ping_t>,
}

impl PingCapability {
    /// Create a ping capability. Declare it as a `static` and list it in your extension's `requires: [ ... ]`.
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
        }
    }
    /// Call the server's `ping()` counter. Returns `None` if the capability was
    /// not populated (preview disabled / not requested) or the server's ABI
    /// version is too old to provide `ping`.
    #[must_use]
    pub fn ping(&self) -> Option<u64> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: a non-null slot was written by the server at load time and
        // points to a 'static `vef_preview_ping_t` the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_PING_ABI_VERSION {
            return None;
        }
        let ping_fn = vtable.ping?;
        // SAFETY: server-provided no-argument function pointer.
        Some(unsafe { ping_fn() })
    }
}

impl Capability for &'static PingCapability {
    fn request(self) -> RequiredCapability {
        RequiredCapability {
            name: VEF_PREVIEW_PING_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: std::ptr::null(),
            capability_config: std::ptr::null(),
        }
    }
}
