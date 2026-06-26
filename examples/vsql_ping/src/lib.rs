use villagesql::preview::ping::PingCapability;
use villagesql::{InValue, VdfReturn};

/// SQL: vsql_ping.ping() -> INT
///
/// Calls the server-provided `vsql::preview::ping` capability and returns its
/// monotonically incrementing counter. Returns NULL if the capability was not
/// populated (e.g. `vsql_allow_preview_extensions` is OFF).
fn ping_impl(_args: &[InValue]) -> VdfReturn {
    match PingCapability::ping() {
        Some(v) => VdfReturn::int(v as i64),
        None => VdfReturn::null(),
    }
}

villagesql::extension! {
    funcs: [
        villagesql::func!(ping_impl, "ping", [] -> villagesql::Type::Int),
    ],
    capabilities: [
        PingCapability::request(),
    ]
}
