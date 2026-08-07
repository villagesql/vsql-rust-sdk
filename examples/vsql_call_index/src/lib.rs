use villagesql::{PrerunArgs, PrerunResult, VdfReturn};

/// Per-statement state: how many times the row function has been called.
struct CallCounter {
    n: i64,
}

/// prerun: allocate the counter at zero. Runs once, before the first row.
fn call_index_prerun(_args: PrerunArgs, out: PrerunResult<CallCounter>) {
    out.set_state(CallCounter { n: 0 });
}

/// row: bump the counter and return its new value. Runs once per row.
fn call_index(state: &mut CallCounter) -> VdfReturn {
    state.n += 1;
    VdfReturn::int(state.n)
}

villagesql::extension! {
    funcs: [
        villagesql::func!(call_index, "call_index", [] -> villagesql::Type::Int,
            state: CallCounter, prerun: call_index_prerun),
    ]
}
