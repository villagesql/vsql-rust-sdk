use villagesql::preview::ping::PingCapability;
use villagesql::{InValue, VdfReturn};

/// The ping capability instance. Declare it as a `static` so it lives for the whole
/// time the extension is loaded; the server populates it at registration.
static PING: PingCapability = PingCapability::new();

/// SQL: `vsql_ping.ping()` -> INT
///
/// Calls the server-provided `vsql::preview::ping` capability and returns its
/// monotonically incrementing counter. Returns NULL if the capability was not
/// populated (e.g. `vsql_allow_preview_extensions` is OFF).
#[allow(clippy::cast_possible_wrap)]
fn ping_impl(_args: &[InValue]) -> VdfReturn {
    match PING.ping() {
        Some(v) => VdfReturn::int(v as i64),
        None => VdfReturn::null(),
    }
}

villagesql::extension! {
    funcs: [
        villagesql::func!(ping_impl, "ping", [] -> villagesql::Type::Int),
    ],
    requires: [
        &PING,
    ]
}
