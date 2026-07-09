//! Idiomatic wrapper for the `vsql::sys_var` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/sys_var.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_preview_sys_var_t, vef_sys_var_desc_t, vef_sys_var_descriptor_list_t,
    vef_sys_var_on_change_func_t, VEF_PREVIEW_SYS_VAR_ABI_VERSION, VEF_PREVIEW_SYS_VAR_NAME,
};

// bindgen names the anonymous value union and its arms in sys_var.h as
// `vef_sys_var_desc_t__bindgen_ty_1[__bindgen_ty_N]` (because they're unnamed
// in C). Those synthesized names are not stable across header edits, so alias
// them to readable names here. `request()` uses the aliases, and a future
// renumber is a one-line fix confined to this block. Ideal long-term fix:
// name the union in the server header.
use crate::sys::{
    vef_sys_var_desc_t__bindgen_ty_1 as SysVarValue,
    vef_sys_var_desc_t__bindgen_ty_1__bindgen_ty_1 as SysVarBool,
    vef_sys_var_desc_t__bindgen_ty_1__bindgen_ty_2 as SysVarInt,
    vef_sys_var_desc_t__bindgen_ty_1__bindgen_ty_4 as SysVarStr,
    vef_var_type_t_VEF_VAR_BOOL as VEF_VAR_BOOL, vef_var_type_t_VEF_VAR_INT as VEF_VAR_INT,
    vef_var_type_t_VEF_VAR_STR as VEF_VAR_STR,
};

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
                        SysVarValue {
                            boolean: SysVarBool {
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
                        SysVarValue {
                            integer: SysVarInt {
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
                        SysVarValue {
                            str_: SysVarStr {
                                value_ptr,
                                def_val: default.as_ptr(),
                            },
                        },
                    )
                }
            };

            let desc = vef_sys_var_desc_t {
                name: name.as_ptr(),
                comment: comment.as_ptr(),
                type_,
                on_change: *on_change,
                __bindgen_anon_1: value,
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
