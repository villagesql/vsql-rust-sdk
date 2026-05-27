# VillageSQL Rust SDK

Write custom SQL functions (VDFs) for VillageSQL in safe Rust. The SDK handles all FFI marshaling so you work entirely in ordinary Rust types.

## Crates

| Crate | Description |
|-------|-------------|
| [`villagesql`](vsql/README.md) | Safe Rust SDK for writing VDF extension functions |
| [`cargo-vsql`](cargo-vsql/README.md) | Cargo subcommand for packaging and testing extensions |
| [`vsql-sys`](vsql-sys/README.md) | Raw FFI bindings (used internally by `vsql`) |

## Prerequisites

- [Rust toolchain](https://rustup.rs) (stable)
- `cargo-vsql` installed (see [cargo-vsql README](cargo-vsql/README.md))
- VillageSQL build directory (for `install` and `test` commands)

## Quick start

### 1. Install cargo-vsql

```sh
cargo install cargo-vsql
```

### 2. Create a new extension crate

```sh
cargo new --lib my-extension
cd my-extension
```

Add to `Cargo.toml`:

```toml
[workspace]
members = ["."]
resolver = "2"

[lib]
crate-type = ["cdylib"]

[dependencies]
villagesql = "0.1"
```

### 3. Write your function

```rust
use villagesql::{InValue, VdfReturn};

fn my_func(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::String(s)) => VdfReturn::string(s.to_uppercase()),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("my_func: expected a STRING argument"),
    }
}

villagesql::extension! {
    funcs: [
        villagesql::func!(my_func, "my_func", [villagesql::Type::String] -> villagesql::Type::String),
    ]
}
```

### 4. Add a manifest

Create `manifest.json` next to `Cargo.toml`:

```json
{
  "name": "my-extension",
  "version": "0.1.0",
  "description": "What your extension does",
  "author": "Your Name",
  "license": "GPL-2.0"
}
```

### 5. Package, install, and test

```sh
export VillageSQL_BUILD_DIR=/path/to/villagesql/build
cargo vsql install
cargo vsql test
```

For the full API reference see the [vsql README](vsql/README.md). For all `cargo vsql` commands see the [cargo-vsql README](cargo-vsql/README.md).

## Examples

- [`examples/vsql_rot13`](examples/vsql_rot13) — minimal string function; a good starting template
- [`examples/vsql_rational`](examples/vsql_rational) — custom type (`n/d` rational numbers) with arithmetic VDFs, demonstrating `custom_type!`, `InValue::Custom`, and `VdfReturn::binary`
