//! Idiomatic wrapper for the `vsql::preview::keyring` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/keyring.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_keyring_result_t, vef_keyring_result_t_VEF_KEYRING_BUF_TOO_SMALL,
    vef_keyring_result_t_VEF_KEYRING_NOT_FOUND, vef_keyring_result_t_VEF_KEYRING_OK,
    vef_keyring_result_t_VEF_KEYRING_UNAVAILABLE, vef_preview_keyring_t,
    VEF_PREVIEW_KEYRING_ABI_VERSION, VEF_PREVIEW_KEYRING_NAME,
};
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against (its `vef_preview_keyring_t` entry
// is registered as "ver-1"). NUL-terminated; `strcmp`'d server-side.
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// Starting buffer size, in bytes, for [`KeyringCapability::read_to_vec`].
/// Matches the C++ SDK.
pub const READ_INITIAL_CAPACITY: usize = 4096;

/// Most server calls [`KeyringCapability::read_to_vec`] makes before giving up.
/// Matches the C++ SDK.
pub const READ_MAX_ATTEMPTS: usize = 3;

/// Why a keyring operation failed.
///
/// Success is reported on the `Ok` side of the returned [`Result`] (`()` for
/// [`KeyringCapability::write`], a `bool` "was it found?" for
/// [`KeyringCapability::read`]), so this enum lists only genuine failures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringError {
    /// The capability was not wired up: the server never populated the vtable,
    /// its function pointer is null, or its ABI version is older than this SDK
    /// requires.
    CapabilityUnavailable,
    /// No keyring component is installed on the server (`VEF_KEYRING_UNAVAILABLE`).
    NoComponent,
    /// The secret is larger than the buffer passed to
    /// [`KeyringCapability::read`] (`VEF_KEYRING_BUF_TOO_SMALL`). `required` is
    /// the buffer size, in bytes, the secret needs. Older servers report this
    /// case as not-found instead.
    BufferTooSmall { required: usize },
    /// Any other keyring failure (`VEF_KEYRING_ERROR`).
    Other,
}

impl std::fmt::Display for KeyringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyringError::CapabilityUnavailable => f.write_str("keyring capability is unavailable"),
            KeyringError::NoComponent => f.write_str("no keyring component is installed"),
            KeyringError::BufferTooSmall { required } => {
                write!(f, "keyring secret needs a {required}-byte buffer")
            }
            KeyringError::Other => f.write_str("keyring operation failed"),
        }
    }
}

impl std::error::Error for KeyringError {}

/// Translate a raw `vef_keyring_result_t` into a Rust result:
/// `Ok(true)` = found, `Ok(false)` = not found, `Err` = a genuine failure.
fn interpret(code: vef_keyring_result_t) -> Result<bool, KeyringError> {
    // Match *guards* (`c if c == …`), not bare patterns: the generated constants
    // start with a lowercase letter, so as a pattern Rust would treat them as a
    // brand-new variable that matches anything — a classic footgun.
    match code {
        c if c == vef_keyring_result_t_VEF_KEYRING_OK => Ok(true),
        c if c == vef_keyring_result_t_VEF_KEYRING_NOT_FOUND => Ok(false),
        c if c == vef_keyring_result_t_VEF_KEYRING_UNAVAILABLE => Err(KeyringError::NoComponent),
        _ => Err(KeyringError::Other),
    }
}

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

    // The server-populated vtable, if the capability is available and new
    // enough. Centralizes the one unavoidable raw-pointer dereference.
    fn vtable(&self) -> Result<&vef_preview_keyring_t, KeyringError> {
        let ptr = self.abi_.load(Ordering::Acquire);
        if ptr.is_null() {
            return Err(KeyringError::CapabilityUnavailable);
        }
        // SAFETY: a non-null slot was written by the server at load time and
        // points to a 'static vef_preview_keyring_t the server owns.
        let vtable = unsafe { &*ptr };
        if vtable.version < VEF_PREVIEW_KEYRING_ABI_VERSION {
            return Err(KeyringError::CapabilityUnavailable);
        }
        Ok(vtable)
    }

    /// Read a secret for `data_id` into `buf`.
    ///
    /// # Returns
    /// - `Ok(Some(n))` — a secret was found; it is the first `n` bytes of `buf`.
    /// - `Ok(None)` — no secret exists for `data_id` (a normal outcome).
    ///
    /// # Errors
    /// - [`KeyringError::CapabilityUnavailable`] if the capability was not wired
    ///   up (or its ABI is older than this SDK requires).
    /// - [`KeyringError::NoComponent`] if no keyring component is installed.
    /// - [`KeyringError::BufferTooSmall`] if the secret does not fit in `buf`;
    ///   `required` is the size to retry with. Pass an empty `buf` to query the
    ///   size. Older servers return `Ok(None)` for this case instead.
    /// - [`KeyringError::Other`] for any other keyring failure.
    pub fn read(
        &self,
        data_id: &CStr,
        auth_id: Option<&CStr>,
        buf: &mut [u8],
    ) -> Result<Option<usize>, KeyringError> {
        let vtable = self.vtable()?;
        let Some(read_fn) = vtable.read else {
            return Err(KeyringError::CapabilityUnavailable);
        };
        let auth_ptr = auth_id.map_or(std::ptr::null(), CStr::as_ptr);
        let mut out_len: usize = 0;
        // SAFETY: read_fn is a server-provided function-pointer; data_id/auth_id
        // are valid NUL-terminated C strings, buf is writeable for buf.len()
        // bytes, and out_len is a valid writeable pointer - all valid for the call.
        let code = unsafe {
            read_fn(
                data_id.as_ptr(),
                auth_ptr,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
            )
        };
        // Checked here rather than in `interpret`, which never sees `out_len`.
        if code == vef_keyring_result_t_VEF_KEYRING_BUF_TOO_SMALL {
            return Err(KeyringError::BufferTooSmall { required: out_len });
        }
        if interpret(code)? {
            Ok(Some(out_len.min(buf.len())))
        } else {
            Ok(None)
        }
    }

    /// Read a secret for `data_id` into a newly allocated `Vec`, growing the
    /// buffer as needed.
    ///
    /// Starts with a [`READ_INITIAL_CAPACITY`]-byte buffer. If the server
    /// reports [`KeyringError::BufferTooSmall`], it resizes to the required size
    /// and retries, up to [`READ_MAX_ATTEMPTS`] calls in total (the secret can
    /// change between calls). Matches the C++ SDK's `KeyringCapability::read`.
    ///
    /// # Returns
    /// - `Ok(Some(secret))` — a secret was found.
    /// - `Ok(None)` — no secret exists for `data_id`. Older servers also return
    ///   this when the secret is larger than the initial buffer.
    ///
    /// # Errors
    /// The same as [`KeyringCapability::read`], except that
    /// [`KeyringError::BufferTooSmall`] is retried rather than returned. If the
    /// secret still does not fit after the last attempt, returns
    /// [`KeyringError::Other`].
    pub fn read_to_vec(
        &self,
        data_id: &CStr,
        auth_id: Option<&CStr>,
    ) -> Result<Option<Vec<u8>>, KeyringError> {
        let mut buf = vec![0u8; READ_INITIAL_CAPACITY];
        for _ in 0..READ_MAX_ATTEMPTS {
            match self.read(data_id, auth_id, &mut buf) {
                Ok(Some(n)) => {
                    buf.truncate(n);
                    return Ok(Some(buf));
                }
                Ok(None) => return Ok(None),
                Err(KeyringError::BufferTooSmall { required }) => buf.resize(required, 0),
                Err(e) => return Err(e),
            }
        }
        Err(KeyringError::Other)
    }

    /// Write a secret (`data`) to the keyring under `data_id`.
    ///
    /// # Errors
    /// - [`KeyringError::CapabilityUnavailable`] if the capability was not wired
    ///   up (or its ABI is older than this SDK requires).
    /// - [`KeyringError::NoComponent`] if no keyring component is installed.
    /// - [`KeyringError::Other`] for any other keyring failure.
    ///
    pub fn write(
        &self,
        data_id: &CStr,
        auth_id: Option<&CStr>,
        data: &[u8],
    ) -> Result<(), KeyringError> {
        let vtable = self.vtable()?;
        let Some(write_fn) = vtable.write else {
            return Err(KeyringError::CapabilityUnavailable);
        };
        let auth_ptr = auth_id.map_or(std::ptr::null(), CStr::as_ptr);
        // SAFETY: server-provided function pointer; data_id/auth_id
        // are valid NUL-terminated C strings and data is readable for data.len()
        // bytes - all valid for the call.
        let code = unsafe { write_fn(data_id.as_ptr(), auth_ptr, data.as_ptr(), data.len()) };
        // A write never returns NOT_FOUND, so `interpret`'s `Ok` is always
        // success; collapse the "found?" bool to `Ok(())`.
        interpret(code).map(|_| ())
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
