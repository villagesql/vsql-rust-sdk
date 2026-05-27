use villagesql::{InValue, VdfReturn};

/// SQL: rot13(s STRING) -> STRING
///
/// Returns the ROT-13 encoding of the input string. Non-ASCII bytes are passed
/// through unchanged. NULL input produces NULL output.
fn rot13_impl(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::String(s)) => VdfReturn::string(rot13(s)),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("rot13: expected a STRING argument"),
    }
}

fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='m' | 'A'..='M' => (c as u8 + 13) as char,
            'n'..='z' | 'N'..='Z' => (c as u8 - 13) as char,
            _ => c,
        })
        .collect()
}

villagesql::extension! {
    funcs: [
        villagesql::func!(rot13_impl, "rot13", [villagesql::Type::String] -> villagesql::Type::String),
    ]
}
