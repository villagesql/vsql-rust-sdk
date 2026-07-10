//! Preview capabilities — opt-in, versioned access to server internals.
//! Gated server-side by `vsql_allow_preview_extensions` (default OFF).

pub mod ping;
pub mod sys_var;
pub mod thread_worker;

use crate::sys::vef_required_capability_t;
use std::ffi::{c_char, c_void};

/// One entry in a registration's `required_capabilities` array.
///
/// Every pointer must be valid for `'static`. The server reads `name`,
/// `vtable_hash`, and `capability_config_hash` during load, then writes
/// the resolved vtable pointer through `vtable_dest` after `vef_register`
/// returns. `vtable_dest` must point to a `static` slot.
#[derive(Copy, Clone)]
pub struct RequiredCapability {
    /// The name of the required capability. NUL-terminated string.
    pub name: *const c_char,
    /// The hash of the vtable for the required capability.
    pub vtable_hash: *const c_char,
    /// A pointer to the location where the resolved vtable will be written.
    pub vtable_dest: *mut *mut c_void,
    /// The hash of the capability configuration.
    pub capability_config_hash: *const c_char,
    /// Capability specific config struct, or null.
    pub capability_config: *const c_void,
}

/// A capability an extension requests from the server. Implemented for a
/// `&'static` reference to each capability instance, so the `extension!` macro
/// can call `.request()` on the values you list in `requires: [ ... ]`.
pub trait Capability {
    /// Build the registration entry the server resolves at load time.
    #[must_use]
    fn request(self) -> RequiredCapability;
}

impl RequiredCapability {
    /// Converts to the FFI structure.
    #[allow(clippy::wrong_self_convention)] // &self is correct here; we don't want to consume the struct
    pub(crate) fn to_raw(&self) -> vef_required_capability_t {
        vef_required_capability_t {
            name: self.name,
            vtable_hash: self.vtable_hash,
            vtable_dest: self.vtable_dest,
            capability_config_hash: self.capability_config_hash,
            capability_config: self.capability_config,
        }
    }
}
