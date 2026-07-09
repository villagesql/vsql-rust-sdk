//! Idiomatic wrapper for the `vsql::preview::keyring` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/keyring.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_keyring_result_t, vef_preview_keyring_t, VEF_PREVIEW_KEYRING_ABI_VERSION,
    VEF_PREVIEW_KEYRING_NAME,
};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against (its `vef_preview_keyring_t` entry
// is registered as "ver-1"). NUL-terminated; `strcmp`'d server-side.
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// The `vsql::preview::keyring` capability. Declare it as a `static` in your
/// extension and list it via `requires: [ &KEYRING ]`; the server populates it
/// at load time, after which [`KeyringCapability::read`] /
/// [`KeyringCapability::write`] work.
pub struct KeyringCapability {
    /// Slot the server fills with its `vef_preview_keyring_t*` at load time.
    abi_: AtomicPtr<vef_preview_keyring_t>,
}

impl KeyringCapability {
    /// Create a keyring capability. Declare it as a `static` in your extension.
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// Read a secret into `buf` (up to `buf_len` bytes; the actual length is
    /// written to `*out_len`).
    ///
    /// # Returns
    /// - `None` if the capability is unavailable.
    /// - `Some(result)` - a `vef_keyring_result_t` (`VEF_KEYRING_OK` = success).
    ///
    /// # Safety
    /// `data_id`/`auth_id` (or null) must be valid NUL-terminated C strings;
    /// `buf` must be writeable for at least `buf_len` bytes and `out_len` a valid
    /// writeable pointer - all valid for the call.
    #[must_use]
    pub unsafe fn read(
        &self,
        data_id: *const c_char,
        auth_id: *const c_char,
        buf: *mut u8,
        buf_len: usize,
        out_len: *mut usize,
    ) -> Option<vef_keyring_result_t> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: non-null slot written by the server at load time points to a
        //  'static vef_preview_keyring_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_KEYRING_ABI_VERSION {
            return None;
        }
        let read_fn = vtable.read?;
        // SAFETY: server-provided function pointer; pointers valid for the call.
        Some(unsafe { read_fn(data_id, auth_id, buf, buf_len, out_len) })
    }

    /// Write a secret (`data`, `data_len` bytes) to the keyring under `data_id`.
    ///
    /// # Returns
    /// - `None` if the capability is unavailable.
    /// - `Some(result)` - a `vef_keyring_result_t` (`VEF_KEYRING_OK` = success).
    ///
    /// # Safety
    /// `data_id`/`auth_id` (or null) must be valid NUL-terminated C strings;
    /// `data` must be readable for at least `data_len` bytes - all valid for the
    /// call.
    #[must_use]
    pub unsafe fn write(
        &self,
        data_id: *const c_char,
        auth_id: *const c_char,
        data: *const u8,
        data_len: usize,
    ) -> Option<vef_keyring_result_t> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: non-null slot written by the server at load time, points to a
        // 'static vef_preview_keyring_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_KEYRING_ABI_VERSION {
            return None;
        }
        let write_fn = vtable.write?;
        // SAFETY: server-provided function pointer; pointers valid for the call.
        Some(unsafe { write_fn(data_id, auth_id, data, data_len) })
    }
}

impl Capability for &'static KeyringCapability {
    fn request(self) -> RequiredCapability {
        RequiredCapability {
            name: VEF_PREVIEW_KEYRING_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: std::ptr::null(),
            capability_config: std::ptr::null(),
        }
    }
}
