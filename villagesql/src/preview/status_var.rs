//! Idiomatic wrapper for the `vsql::status_var` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/status_var.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_preview_status_var_t, vef_status_var_desc_t, vef_status_var_descriptor_list_t,
    VEF_PREVIEW_STATUS_VAR_NAME,
};
// bindgen names status_var.h's anonymous value union `vef_status_var_desc_t__bindgen_ty_1`
// and prefixes the type-tag constants. Alias them to readable names here. Those
// synthesized names aren't stable across header edits (see sys_var.rs for the full
// rationale. Ideal long term fix is naming the union in the server header).
use crate::sys::{
    vef_status_var_desc_t__bindgen_ty_1 as StatusVarValue,
    vef_status_var_type_t_VEF_STATUS_VAR_DOUBLE as VEF_STATUS_VAR_DOUBLE,
    vef_status_var_type_t_VEF_STATUS_VAR_INT as VEF_STATUS_VAR_INT,
};
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicI64, AtomicPtr, AtomicU64, Ordering};

// Version tags the server matches against (both `"ver-1"`, strcmp'd server-side).
const VTABLE_HASH: &[u8] = b"ver-1\0";
const CONFIG_HASH: &[u8] = b"ver-1\0";

/// An `f64` with atomic access, stored as its IEEE-754 bits in an `AtomicU64`
/// since Rust std has no `AtomicF64`. Every 64-bit pattern is a valid `f64`,
/// so the round-trip is lossless. Layout-identical to `f64`, so `as_ptr()`
/// yields a `*mut f64` the server reads directly.
pub struct AtomicF64(AtomicU64);

impl AtomicF64 {
    #[must_use]
    pub const fn new(v: f64) -> Self {
        Self(AtomicU64::new(v.to_bits()))
    }
    pub fn store(&self, v: f64, order: Ordering) {
        self.0.store(v.to_bits(), order);
    }
    #[must_use]
    pub fn load(&self, order: Ordering) -> f64 {
        f64::from_bits(self.0.load(order))
    }
    fn as_ptr(&self) -> *mut f64 {
        self.0.as_ptr().cast::<f64>()
    }
}

/// One status variable an extension declares. The extension owns the counter (a
/// `'static` atomic); the server reads it live at `SHOW STATUS`.
pub enum StatusVarSpec {
    Int {
        name: &'static CStr,
        value: &'static AtomicI64,
    },
    Double {
        name: &'static CStr,
        value: &'static AtomicF64,
    },
}

/// The `vsql::status_var` capability. Declare it as a `static` and list it via
/// `requires: [&STATUS_VAR]`.
pub struct StatusVarCapability {
    abi_: AtomicPtr<vef_preview_status_var_t>,
    specs: &'static [StatusVarSpec],
}

impl StatusVarCapability {
    /// Create a `status_var` capability declaring `specs`.
    #[must_use]
    pub const fn new(specs: &'static [StatusVarSpec]) -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
            specs,
        }
    }
}

impl Capability for &'static StatusVarCapability {
    /// Build the registration entry declaring this capabiltiy's status variables.
    ///
    /// # Panics
    /// Panics if the number of declared variables exceeds `u32::MAX`.
    fn request(self) -> RequiredCapability {
        // Everything allocated here is leaked with `Box::into_raw`: the server
        // keeps `capability_config` for the extension's lifetime, so it must
        // outlive this call.
        let mut desc_ptrs: Vec<*const vef_status_var_desc_t> = Vec::with_capacity(self.specs.len());
        for spec in self.specs {
            let (name, type_, value_union) = match spec {
                StatusVarSpec::Int { name, value } => (
                    name,
                    VEF_STATUS_VAR_INT,
                    StatusVarValue {
                        integer_ptr: value.as_ptr(),
                    },
                ),
                StatusVarSpec::Double { name, value } => (
                    name,
                    VEF_STATUS_VAR_DOUBLE,
                    StatusVarValue {
                        double_ptr: value.as_ptr(),
                    },
                ),
            };
            let desc = vef_status_var_desc_t {
                name: name.as_ptr(),
                type_,
                __bindgen_anon_1: value_union,
            };
            desc_ptrs.push(Box::into_raw(Box::new(desc)));
        }

        let var_count = u32::try_from(desc_ptrs.len()).expect("variable count exceeds u32");
        let vars_ptr = Box::into_raw(desc_ptrs.into_boxed_slice())
            .cast::<*const vef_status_var_desc_t>()
            .cast_const();

        let list = vef_status_var_descriptor_list_t {
            vars: vars_ptr,
            var_count,
        };
        let list_ptr = Box::into_raw(Box::new(list));

        RequiredCapability {
            name: VEF_PREVIEW_STATUS_VAR_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: CONFIG_HASH.as_ptr().cast::<c_char>(),
            capability_config: list_ptr.cast::<c_void>(),
        }
    }
}
