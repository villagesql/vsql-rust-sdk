#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::all, clippy::pedantic)]

include!("bindings/types.rs");
include!("bindings/preview/ping.rs");
include!("bindings/preview/sys_var.rs");
include!("bindings/preview/status_var.rs");
include!("bindings/preview/keyring.rs");
include!("bindings/preview/thread_worker.rs");
include!("bindings/preview/sql_query.rs");

/// The `VillageSQL` **server** release whose ABI headers are vendored in this
/// repository's `include/` directory.
///
/// This is not this crate's own version, and the two move independently. The server
/// reads it during `vef_register()` and publishes it as the `sdk_version` key of
/// `INFORMATION_SCHEMA.EXTENSION_REGISTRATION.REGISTRATION_JSON`, so it answers
/// "which server SDK was this extension built against?" and nothing else.
///
/// Bump it whenever you re-vendor the headers under `include/`. CI diffs those headers
/// against a real server build, so a header change that needs this bump shows up as a
/// failing job. A server release that changes no header does not, so check this value
/// when the SDK starts tracking a new release.
pub const SDK_VERSION: vef_version_t = vef_version_t {
    major: 0,
    minor: 0,
    patch: 6,
    extra: core::ptr::null(),
};
