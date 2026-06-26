//! ABI definitions for the 'vsql::preview::ping' capability.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/ping.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.
//! Keep this struct byte for byte compatible with the server implementation.

#![allow(non_camel_case_types)]

/// Capability ABI version this SDK snapshot was written against.
pub const VEF_PREVIEW_PING_ABI_VERSION: u32 = 1;

/// Capability name. NUL-terminated string for FFI.
pub const VEF_PREVIEW_PING_NAME: &[u8] = b"vsql::preview::ping\0";

/// `uint64_t (*)(void)` — returns a monotonically incrementing counter.
///
/// Modeled as  `Option<fn>` to match bindgen's nullable-fn-pointer convention.
pub type vef_ping_fn = Option<unsafe extern "C" fn() -> u64>;

/// Server-provided vtable for the ping capability
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_preview_ping_t {
    /// The version of the ABI.
    pub version: u32,
    /// version >= 1
    pub ping: vef_ping_fn,
}

/// Guard the layout against drift from the C header (64-bit alignment).
const _: () = {
    assert!(::std::mem::size_of::<vef_preview_ping_t>() == 16);
    assert!(::std::mem::align_of::<vef_preview_ping_t>() == 8);
    assert!(::std::mem::offset_of!(vef_preview_ping_t, version) == 0);
    assert!(::std::mem::offset_of!(vef_preview_ping_t, ping) == 8);
};

use crate::preview::RequiredCapability;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against (its `vef_preview_ping_t` entry
// is registered as "ver-1"). NUL-terminated; `strcmp`'d server-side.
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// `'static` slot the server populates with its `vef_preview_ping_t*` at load
/// time. `AtomicPtr<T>` is layout-identical to `*mut T`, so handing its address
/// to the server as `vtable_dest` is ABI-sound; the atomic just lets the Rust
/// side read it without a data race.
static PING_VTABLE: AtomicPtr<vef_preview_ping_t> = AtomicPtr::new(std::ptr::null_mut());

/// The `vsql::preview::ping` capability. Declare it in an extension via
/// `capabilities: [ PingCapability::request() ]`, then call
/// [`PingCapability::ping`] at runtime once the server has populated it.
pub struct PingCapability;

impl PingCapability {
    /// Build the registration entry the server resolves at load time.
    #[must_use]
    pub fn request() -> RequiredCapability {
        RequiredCapability {
            name: VEF_PREVIEW_PING_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: PING_VTABLE.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: std::ptr::null(),
            capability_config: std::ptr::null(),
        }
    }

    /// Call the server's `ping()` counter. Returns `None` if the capability was
    /// not populated (preview disabled / not requested) or the server's ABI
    /// version is too old to provide `ping`.
    #[must_use]
    pub fn ping() -> Option<u64> {
        let vtable = PING_VTABLE.load(Ordering::Acquire);
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
