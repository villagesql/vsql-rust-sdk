//! ABI definitions for the '`vsql::sys_var`' capability.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/sys_var.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.
//! Keep this struct byte for byte compatible with the server implementation.

#![allow(non_camel_case_types)]

/// Capability ABI version this SDK snapshot was written against.
pub const VEF_PREVIEW_SYS_VAR_ABI_VERSION: u32 = 1;

/// Capability name. NUL-terminated string for FFI.
pub const VEF_PREVIEW_SYS_VAR_NAME: &[u8] = b"vsql::sys_var\0";

/// Server-provided vtable for the `sys_var` capability
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_preview_sys_var_t {
    /// The ABI version.
    pub version: u32,
    /// The 'get' function.
    pub get: vef_sys_var_get_func_t,
    /// The 'set' function.
    pub set: vef_sys_var_set_func_t,
}

/// Guard the layout against drift from the C header.
const _: () = {
    assert!(::std::mem::size_of::<vef_preview_sys_var_t>() == 24);
    assert!(::std::mem::align_of::<vef_preview_sys_var_t>() == 8);
    assert!(::std::mem::offset_of!(vef_preview_sys_var_t, version) == 0);
    assert!(::std::mem::offset_of!(vef_preview_sys_var_t, get) == 8);
    assert!(::std::mem::offset_of!(vef_preview_sys_var_t, set) == 16);
};

use crate::preview::{Capability, RequiredCapability};
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// `capability_config` version tag the server matches against.
const CONFIG_HASH: &[u8] = b"ver-1\0";

/// One system variable an extension wants to declare.
pub enum SysVarSpec {
    Bool {
        name: &'static CStr,
        comment: &'static CStr,
        default: bool,
        on_change: vef_sys_var_on_change_func_t, // optional callback for when the value changes
    },
    Int {
        name: &'static CStr,
        comment: &'static CStr,
        default: i64,
        min: i64,
        max: i64,
        on_change: vef_sys_var_on_change_func_t,
    },
    Str {
        name: &'static CStr,
        comment: &'static CStr,
        default: &'static CStr,
        on_change: vef_sys_var_on_change_func_t,
    },
}
/// The `vsql::sys_var` capability. Declare it as a `static` in your extension
/// and list it via `requires: [ &SYS_VAR ] `; the server populates it at load
/// time, after which [`SysVarCapability::get`] and [`SysVarCapability::set`] work
/// and the declared variables are registered.
pub struct SysVarCapability {
    /// Slot the server fills with its `vef_preview_sys_var_t*` vtable at load time.
    /// `AtomicPtr<T>` is layout-identical to `*mut T`, so handing its address to
    /// the server as `vtable_dest` is safe. The atomic makes it so the Rust side
    /// can read it without a data race.
    abi_: AtomicPtr<vef_preview_sys_var_t>,
    /// The system variables this capability declares.
    specs: &'static [SysVarSpec],
}

impl SysVarCapability {
    /// Create a `sys_var` capability declaring `specs`. Declare it as a `static`
    /// in your extension and list it via `requires: [&SYS_VAR]`.
    #[must_use]
    pub const fn new(specs: &'static [SysVarSpec]) -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
            specs,
        }
    }

    /// Read a system variable owned by `component_name` (an extension name),
    /// via the `vsql::sys_var` capability.
    ///
    /// # Returns
    /// - `None` — the capability is unavailable (preview disabled / not
    ///   requested, or the server's ABI is too old / `get` unset).
    /// - `Some(false)` — **success**: the server wrote a newly `malloc`'d,
    ///   NUL-terminated value string into `*val` and its length into `*val_len`.
    /// - `Some(true)` — the server reported an error; `*val`/`*val_len` are left
    ///   untouched.
    ///
    /// (Note the inverted C convention: `false` means success.)
    ///
    /// # Safety
    /// - `component_name` and `name` must be valid, NUL-terminated C strings;
    ///   `val` and `val_len` must be valid, writable pointers — all valid for
    ///   the duration of the call.
    /// - `*val` is written **only** on `Some(false)`; you must then release it
    ///   with the C `free()`. On `Some(true)` or `None`, do not read or free
    ///   `*val` — it was not written.
    #[must_use]
    pub unsafe fn get(
        &self,
        component_name: *const c_char,
        name: *const c_char,
        val: *mut *mut c_void,
        val_len: *mut usize,
    ) -> Option<bool> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: non-null slot written by the server at load time, points to a
        // 'static vef_preview_sys_var_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_SYS_VAR_ABI_VERSION {
            return None;
        }
        let get_fn = vtable.get?;
        // SAFETY: server-provided function pointer; pointers valid for the call.
        Some(unsafe { get_fn(component_name, name, val, val_len) })
    }

    /// Set a system variable owned by `component_name` to `val`, via the
    /// `vsql::sys_var` capability.
    ///
    /// `scope` selects persistence: `null` = running value only, `"PERSIST"` =
    /// running value + persisted, `"PERSIST_ONLY"` = persisted (applies on
    /// restart).
    ///
    /// # Returns
    /// - `None` — the capability is unavailable (preview disabled / not
    ///   requested, or the server's ABI is too old / `set` unset).
    /// - `Some(false)` — **success** (note the inverted C convention).
    /// - `Some(true)` — the server reported an error.
    ///
    /// # Safety
    /// `component_name`, `name`, and `val` must be valid, NUL-terminated C
    /// strings; `scope` must be null or a valid NUL-terminated C string — all
    /// valid for the duration of the call.
    #[must_use]
    pub unsafe fn set(
        &self,
        component_name: *const c_char,
        name: *const c_char,
        scope: *const c_char,
        val: *const c_char,
    ) -> Option<bool> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: non-null slot written by the server at load time, points to a
        // 'static vef_preview_sys_var_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_SYS_VAR_ABI_VERSION {
            return None;
        }
        let set_fn = vtable.set?;
        // SAFETY: server-provided function pointer; pointers valid for the call.
        Some(unsafe { set_fn(component_name, name, scope, val) })
    }
}

impl Capability for &'static SysVarCapability {
    /// Build the registration entry the server resolves at load time, declaring
    /// this capability's system variables.
    ///
    /// # Panics
    /// Panics if the number of declared variables exceeds `u32::MAX`.
    fn request(self) -> RequiredCapability {
        // Everything allocated here (value storage, descriptors, the pointer
        // array, and the list) is deliberately leaked with `Box::into_raw`: the
        // server keeps `capability_config` for the extension's lifetime, so it
        // must outlive this call and is intentionally NOT freed in
        // `free_registration`.
        let mut desc_ptrs: Vec<*const vef_sys_var_desc_t> = Vec::with_capacity(self.specs.len());
        for spec in self.specs {
            // Each arm leaks storage for the variable's current value and yields
            // the type-specific (type_, value) pair. The shared descriptor build,
            // leak, and push happen once below.
            let (name, comment, on_change, type_, value) = match spec {
                SysVarSpec::Bool {
                    name,
                    comment,
                    default,
                    on_change,
                } => {
                    let value_ptr: *mut bool = Box::into_raw(Box::new(*default));
                    (
                        name,
                        comment,
                        on_change,
                        VEF_VAR_BOOL,
                        vef_sys_var_value_t {
                            boolean: vef_sys_var_bool_t {
                                value_ptr,
                                def_val: *default,
                            },
                        },
                    )
                }
                SysVarSpec::Int {
                    name,
                    comment,
                    default,
                    min,
                    max,
                    on_change,
                } => {
                    let value_ptr: *mut i64 = Box::into_raw(Box::new(*default));
                    (
                        name,
                        comment,
                        on_change,
                        VEF_VAR_INT,
                        vef_sys_var_value_t {
                            integer: vef_sys_var_int_t {
                                value_ptr,
                                def_val: *default,
                                min_val: *min,
                                max_val: *max,
                            },
                        },
                    )
                }
                SysVarSpec::Str {
                    name,
                    comment,
                    default,
                    on_change,
                } => {
                    let value_ptr: *mut *mut c_char =
                        Box::into_raw(Box::new(std::ptr::null_mut::<c_char>()));
                    (
                        name,
                        comment,
                        on_change,
                        VEF_VAR_STR,
                        vef_sys_var_value_t {
                            str_: vef_sys_var_str_t {
                                value_ptr,
                                def_val: default.as_ptr().cast::<c_char>(),
                            },
                        },
                    )
                }
            };

            let desc = vef_sys_var_desc_t {
                name: name.as_ptr().cast::<c_char>(),
                comment: comment.as_ptr().cast::<c_char>(),
                type_,
                on_change: *on_change,
                value,
            };
            let desc_ptr: *const vef_sys_var_desc_t = Box::into_raw(Box::new(desc));
            desc_ptrs.push(desc_ptr);
        }

        // Leak the array of pointers. Get its base pointer and count.
        let var_count = u32::try_from(desc_ptrs.len()).expect("variable count exceeds u32");
        let vars_ptr = Box::into_raw(desc_ptrs.into_boxed_slice())
            .cast::<*const vef_sys_var_desc_t>()
            .cast_const();

        // Leak the descriptor list.
        let list = vef_sys_var_descriptor_list_t {
            vars: vars_ptr,
            var_count,
        };
        let list_ptr: *const vef_sys_var_descriptor_list_t = Box::into_raw(Box::new(list));

        RequiredCapability {
            name: VEF_PREVIEW_SYS_VAR_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: CONFIG_HASH.as_ptr().cast::<c_char>(),
            capability_config: list_ptr.cast::<c_void>(),
        }
    }
}

pub type vef_sys_var_get_func_t = Option<
    unsafe extern "C" fn(
        component_name: *const c_char,
        name: *const c_char,
        val: *mut *mut c_void,
        val_len: *mut usize,
    ) -> bool,
>;
pub type vef_sys_var_set_func_t = Option<
    unsafe extern "C" fn(
        component_name: *const c_char,
        name: *const c_char,
        scope: *const c_char,
        val: *const c_char,
    ) -> bool,
>;
// ── vef_var_type_t: which value type a sys var holds ──────────────────────────
pub type vef_var_type_t = u32;
pub const VEF_VAR_BOOL: vef_var_type_t = 0;
pub const VEF_VAR_INT: vef_var_type_t = 1;
pub const VEF_VAR_DOUBLE: vef_var_type_t = 2;
pub const VEF_VAR_STR: vef_var_type_t = 3;

// ── Union arms: one struct per value type ─────────────────────────────────────
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_sys_var_bool_t {
    pub value_ptr: *mut bool,
    pub def_val: bool,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_bool_t>() == 16);
    assert!(::std::mem::align_of::<vef_sys_var_bool_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_bool_t, value_ptr) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_bool_t, def_val) == 8);
};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_sys_var_int_t {
    pub value_ptr: *mut i64,
    pub def_val: i64,
    pub min_val: i64,
    pub max_val: i64,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_int_t>() == 32);
    assert!(::std::mem::align_of::<vef_sys_var_int_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_int_t, value_ptr) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_int_t, def_val) == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_int_t, min_val) == 16);
    assert!(::std::mem::offset_of!(vef_sys_var_int_t, max_val) == 24);
};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_sys_var_dbl_t {
    pub value_ptr: *mut f64,
    pub def_val: f64,
    pub min_val: f64,
    pub max_val: f64,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_dbl_t>() == 32);
    assert!(::std::mem::align_of::<vef_sys_var_dbl_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_dbl_t, value_ptr) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_dbl_t, def_val) == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_dbl_t, min_val) == 16);
    assert!(::std::mem::offset_of!(vef_sys_var_dbl_t, max_val) == 24);
};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_sys_var_str_t {
    pub value_ptr: *mut *mut c_char,
    pub def_val: *const c_char,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_str_t>() == 16);
    assert!(::std::mem::align_of::<vef_sys_var_str_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_str_t, value_ptr) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_str_t, def_val) == 8);
};

// ── The union: a sys var's type-specific storage. Only the arm matching
//    `type_` is valid; reading any union field is `unsafe`. ─────────────────────
#[repr(C)]
#[derive(Copy, Clone)]
pub union vef_sys_var_value_t {
    pub boolean: vef_sys_var_bool_t,
    pub integer: vef_sys_var_int_t,
    pub dbl: vef_sys_var_dbl_t,
    pub str_: vef_sys_var_str_t,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_value_t>() == 32);
    assert!(::std::mem::align_of::<vef_sys_var_value_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_value_t, boolean) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_value_t, integer) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_value_t, dbl) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_value_t, str_) == 0);
};

// ── One variable's descriptor (extension fills this in) ───────────────────────
#[repr(C)]
#[derive(Copy, Clone)]
pub struct vef_sys_var_desc_t {
    pub name: *const c_char,
    pub comment: *const c_char,
    pub type_: vef_var_type_t,
    pub on_change: vef_sys_var_on_change_func_t,
    pub value: vef_sys_var_value_t,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_desc_t>() == 64);
    assert!(::std::mem::align_of::<vef_sys_var_desc_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_desc_t, name) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_desc_t, comment) == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_desc_t, type_) == 16);
    assert!(::std::mem::offset_of!(vef_sys_var_desc_t, on_change) == 24);
    assert!(::std::mem::offset_of!(vef_sys_var_desc_t, value) == 32);
};

// ── The list of descriptors handed to the server via capability_config ────────
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct vef_sys_var_descriptor_list_t {
    pub vars: *const *const vef_sys_var_desc_t,
    pub var_count: u32,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_descriptor_list_t>() == 16);
    assert!(::std::mem::align_of::<vef_sys_var_descriptor_list_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_descriptor_list_t, vars) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_descriptor_list_t, var_count) == 8);
};

#[repr(C)]
#[derive(Copy, Clone)]
pub union vef_sys_var_change_value_t {
    pub bool_val: bool,
    pub int_val: i64,
    pub dbl_val: f64,
    pub str_val: *const c_char,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_change_value_t>() == 8);
    assert!(::std::mem::align_of::<vef_sys_var_change_value_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_change_value_t, bool_val) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_change_value_t, int_val) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_change_value_t, dbl_val) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_change_value_t, str_val) == 0);
};

// What the server hands your on_change callback
#[repr(C)]
#[derive(Copy, Clone)]
pub struct vef_sys_var_change_t {
    pub var_name: *const c_char,
    pub type_: vef_var_type_t,
    pub value: vef_sys_var_change_value_t,
}
const _: () = {
    assert!(::std::mem::size_of::<vef_sys_var_change_t>() == 24);
    assert!(::std::mem::align_of::<vef_sys_var_change_t>() == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_change_t, var_name) == 0);
    assert!(::std::mem::offset_of!(vef_sys_var_change_t, type_) == 8);
    assert!(::std::mem::offset_of!(vef_sys_var_change_t, value) == 16);
};

/// Callback the server calls after a system variable changes. Runs on the server's thread.
pub type vef_sys_var_on_change_func_t = Option<unsafe extern "C" fn(*const vef_sys_var_change_t)>;
