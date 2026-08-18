use villagesql::{InValue, VdfReturn};

/// Accumulator for `agg_sum`: the running total for the current group.
#[derive(Default)]
struct SumState {
    total: i64,
    seen: bool,
}

/// clear: reset the total at the start of each group.
fn agg_sum_clear(state: &mut SumState) {
    state.total = 0;
    state.seen = false;
}

/// accumulate: fold one row's int into the running total.
fn agg_sum_acc(state: &mut SumState, args: &[InValue]) {
    if let Some(InValue::Int(n)) = args.first() {
        state.total += *n;
        state.seen = true;
    }
}

/// result: emit the group's total once every row has been folded in.
fn agg_sum_result(state: &SumState) -> VdfReturn {
    if state.seen {
        VdfReturn::int(state.total)
    } else {
        VdfReturn::Null
    }
}

villagesql::extension! {
    funcs: [
        villagesql::agg_func!(agg_sum_result, "agg_sum",
            [villagesql::Type::Int] -> villagesql::Type::Int,
            state: SumState, clear: agg_sum_clear, accumulate: agg_sum_acc),
    ]
}
