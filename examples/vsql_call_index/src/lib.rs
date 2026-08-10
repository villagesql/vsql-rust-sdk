use villagesql::{InValue, PrerunArgs, PrerunResult, VdfReturn};

/// Per-statement state: how many times the row function has been called.
struct CallCounter {
    n: i64,
}

/// Per-statement state for `running_concat`: the text accumulated so far.
struct Concat {
    acc: String,
}

/// prerun: allocate the counter at zero. Runs once, before the first row.
fn call_index_prerun(_args: PrerunArgs, out: PrerunResult<CallCounter>) {
    out.set_state(CallCounter { n: 0 });
}

/// row: bump the counter and return its new value. Runs once per row.
fn call_index(state: &mut CallCounter, _args: &[InValue]) -> VdfReturn {
    state.n += 1;
    VdfReturn::int(state.n)
}

/// prerun: start empty and request a result buffer bigger than the 256-byte default,
/// since the running concat grows past it.
fn running_concat_prerun(_args: PrerunArgs, mut out: PrerunResult<Concat>) {
    out.request_buffer_size(4096);
    out.set_state(Concat { acc: String::new() });
}

/// row: append this row's string to the accumulator, return it so far.
/// Uses both the state and the row value.
fn running_concat(state: &mut Concat, args: &[InValue]) -> VdfReturn {
    if let Some(InValue::String(s)) = args.first() {
        state.acc.push_str(s);
    }
    VdfReturn::string(state.acc.clone())
}

villagesql::extension! {
    funcs: [
        villagesql::func!(call_index, "call_index", [] -> villagesql::Type::Int,
            state: CallCounter, prerun: call_index_prerun),
        villagesql::func!(running_concat, "running_concat", [villagesql::Type::String] ->
            villagesql::Type::String, state: Concat, prerun: running_concat_prerun),
    ]
}
