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
#[derive(Copy, Clone, Debug)]
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

use crate::preview::RequiredCapability;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

// ABI version tag the server matches against
const VTABLE_HASH: &[u8] = b"ver-1\0";

/// `capability_config` version tag the server matches against.
const CONFIG_HASH: &[u8] = b"ver-1\0";

/// `'static` slot the server populates with its `vef_preview_sys_var_t*` at load
/// time. `AtomicPtr<T>` is layout-identical to `*mut T`, so handing its address
/// to the server as `vtable_dest` is ABI-sound; the atomic just lets the Rust
/// side read it without a data race.
static SYS_VAR_VTABLE: AtomicPtr<vef_preview_sys_var_t> = AtomicPtr::new(std::ptr::null_mut());

/// One system variable an extension wants to declare.
pub enum SysVarSpec {
    Bool {
        name: &'static [u8],    // NUL-terminated, e.g. b"enabled\0"
        comment: &'static [u8], // NUL-terminated, e.g. b"Enable the feature\0"
        default: bool,
        on_change: vef_sys_var_on_change_func_t, // optional callback for when the value changes
    },
    Int {
        name: &'static [u8],
        comment: &'static [u8],
        default: i64,
        min: i64,
        max: i64,
        on_change: vef_sys_var_on_change_func_t,
    },
    Str {
        name: &'static [u8],
        comment: &'static [u8],
        default: &'static [u8], // NUL-terminated
        on_change: vef_sys_var_on_change_func_t,
    },
}

pub struct SysVarCapability;

impl SysVarCapability {
    /// Build the registration entry the server resolves at load time.
    /// Declares the extension's system variables as `specs`.
    ///
    /// # Panics
    /// Panics if the number of declared variables exceeds `u32::MAX`.
    #[must_use]
    pub fn request(specs: &[SysVarSpec]) -> RequiredCapability {
        let mut desc_ptrs: Vec<*const vef_sys_var_desc_t> = Vec::with_capacity(specs.len());
        for spec in specs {
            match spec {
                SysVarSpec::Bool {
                    name,
                    comment,
                    default,
                    on_change,
                } => {
                    // Leak storage for the current value pointer
                    let value_ptr: *mut bool = Box::into_raw(Box::new(*default));
                    let desc = vef_sys_var_desc_t {
                        name: name.as_ptr().cast::<c_char>(),
                        comment: comment.as_ptr().cast::<c_char>(),
                        type_: VEF_VAR_BOOL,
                        on_change: *on_change,
                        value: vef_sys_var_value_t {
                            boolean: vef_sys_var_bool_t {
                                value_ptr,
                                def_val: *default,
                            },
                        },
                    };

                    // Leak the descriptor; keep a *const pointer to it.
                    let desc_ptr: *const vef_sys_var_desc_t = Box::into_raw(Box::new(desc));
                    desc_ptrs.push(desc_ptr);
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
                    let desc = vef_sys_var_desc_t {
                        name: name.as_ptr().cast::<c_char>(),
                        comment: comment.as_ptr().cast::<c_char>(),
                        type_: VEF_VAR_INT,
                        on_change: *on_change,
                        value: vef_sys_var_value_t {
                            integer: vef_sys_var_int_t {
                                value_ptr,
                                def_val: *default,
                                min_val: *min,
                                max_val: *max,
                            },
                        },
                    };
                    let desc_ptr: *const vef_sys_var_desc_t = Box::into_raw(Box::new(desc));
                    desc_ptrs.push(desc_ptr);
                }
                SysVarSpec::Str {
                    name,
                    comment,
                    default,
                    on_change,
                } => {
                    // Leak storage for the current value pointer
                    let value_ptr: *mut *mut c_char =
                        Box::into_raw(Box::new(std::ptr::null_mut::<c_char>()));
                    let desc = vef_sys_var_desc_t {
                        name: name.as_ptr().cast::<c_char>(),
                        comment: comment.as_ptr().cast::<c_char>(),
                        type_: VEF_VAR_STR,
                        on_change: *on_change,
                        value: vef_sys_var_value_t {
                            str_: vef_sys_var_str_t {
                                value_ptr,
                                def_val: default.as_ptr().cast::<c_char>(),
                            },
                        },
                    };
                    let desc_ptr: *const vef_sys_var_desc_t = Box::into_raw(Box::new(desc));
                    desc_ptrs.push(desc_ptr);
                }
            }
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
            vtable_dest: SYS_VAR_VTABLE.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: CONFIG_HASH.as_ptr().cast::<c_char>(),
            capability_config: list_ptr.cast::<c_void>(),
        }
    }

    #[must_use]
    /// Read a system variable owned by `component_name` (an extension name),
    /// via the `vsql::sys_var` capability.
    ///
    /// On success the server allocates a NUL-terminated value string into `*val`
    /// (length into `*val_len`); the caller must free it with the C `free()`.
    ///
    /// Returns `None` if the capability is unavailable; otherwise `Some(false)`
    /// on success and `Some(true)` on error (the C convention is inverted).
    ///
    /// # Safety
    /// `component_name`/`name` must be valid NUL-terminated C strings, and
    /// `val`/`val_len` valid writable pointers, all valid for the call.
    pub unsafe fn get(
        component_name: *const c_char,
        name: *const c_char,
        val: *mut *mut c_void,
        val_len: *mut usize,
    ) -> Option<bool> {
        let vtable = SYS_VAR_VTABLE.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // Safety: non-null slot written by the server at load time, points to a
        // 'static vef_preview_sys_var_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_SYS_VAR_ABI_VERSION {
            return None;
        }
        let get_fn = vtable.get?;
        // Safety: server-provided function pointer; pointers valid for the call.
        Some(unsafe { get_fn(component_name, name, val, val_len) })
    }

    #[must_use]
    /// Set a system variable owned by `component_name` to `val`, via the
    /// `vsql::sys_var` capability.
    ///
    /// `scope` is null (running value only), `"PERSIST"` (running + persisted),
    /// or `"PERSIST_ONLY"` (persisted, applies on restart).
    ///
    /// Returns `None` if the capability is unavailable; otherwise `Some(false)`
    /// on success and `Some(true)` on error (the C convention is inverted).
    ///
    /// # Safety
    /// `component_name`/`name`/`val` must be valid NUL-terminated C strings and
    /// `scope` null or a valid NUL-terminated C string, all valid for the call.
    pub unsafe fn set(
        component_name: *const c_char,
        name: *const c_char,
        scope: *const c_char,
        val: *const c_char,
    ) -> Option<bool> {
        let vtable = SYS_VAR_VTABLE.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // Safety: non-null slot written by the server at load time, points to a
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
