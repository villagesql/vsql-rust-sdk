//! Every combination and order of setting keys compiles, on `func!`,
//! `agg_func!`, and `varargs_func!`. This is a compile-time check: if any
//! call below stops matching a macro arm, `cargo test` fails to build.

use villagesql::{InValue, PrerunArgs, PrerunResult, Type, VdfReturn};

fn plain(_args: &[InValue]) -> VdfReturn {
    VdfReturn::null()
}

fn setup(_args: PrerunArgs, out: PrerunResult<i64>) {
    out.set_state(0);
}

fn counted(_state: &mut i64, _args: &[InValue]) -> VdfReturn {
    VdfReturn::null()
}

fn clear(_state: &mut i64) {}

fn teardown(_state: &mut i64) {}

fn accumulate(_state: &mut i64, _args: &[InValue]) {}

// `agg_func!` hands the result function `&State`, even for a small `Copy` state.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn total(_state: &i64) -> VdfReturn {
    VdfReturn::null()
}

#[test]
fn setting_combos_compile() {
    // func!: no settings, each one alone, both in either order.
    let _ = villagesql::func!(plain, "a", [] -> Type::String);
    let _ = villagesql::func!(plain, "b", [] -> Type::String, buffer_size: 64);
    let _ = villagesql::func!(plain, "c", [] -> Type::String, deterministic: true);
    let _ = villagesql::func!(plain, "d", [] -> Type::String,
        buffer_size: 64, deterministic: true);
    let _ = villagesql::func!(plain, "e", [] -> Type::String,
        deterministic: true, buffer_size: 64);

    // A setting value can be any expression, not just a literal.
    let _ = villagesql::func!(plain, "f", [] -> Type::String, buffer_size: 8 * 1024);

    // State keys first, in order, then settings in any order.
    let _ = villagesql::func!(counted, "g", [] -> Type::Int,
        state: i64, prerun: setup, deterministic: true);
    let _ = villagesql::func!(counted, "h", [] -> Type::Int,
        state: i64, prerun: setup, postrun: teardown, buffer_size: 32, deterministic: true);

    // agg_func!
    let _ = villagesql::agg_func!(total, "i", [Type::Int] -> Type::Int,
        state: i64, clear: clear, accumulate: accumulate);
    let _ = villagesql::agg_func!(total, "j", [Type::Int] -> Type::Int,
        state: i64, clear: clear, accumulate: accumulate,
        deterministic: true, buffer_size: 16);

    // varargs_func!
    let _ = villagesql::varargs_func!(plain, "k", [..] -> Type::String);
    let _ = villagesql::varargs_func!(plain, "l", [..] -> Type::String,
        buffer_size: 64, deterministic: true);
}
