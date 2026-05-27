# VillageSQL Rust SDK

Write custom SQL functions (VDFs) for VillageSQL in safe Rust. The SDK handles all FFI marshaling so you work entirely in ordinary Rust types.

## Prerequisites

- Rust toolchain (stable)
- `cargo-vsql` installed (see [Installing cargo-vsql](#installing-cargo-vsql))
- VillageSQL build directory (for `install` and `test` commands)

## Installing cargo-vsql

From the root of this repository:

```sh
cargo install --path cargo-vsql
```

## Quick start

The complete example lives in [`examples/vsql_rot13`](examples/vsql_rot13). The steps below walk through it.

### 1. Create a new crate

```sh
cargo new --lib my-extension
cd my-extension
```

Set the crate type in `Cargo.toml`:

```toml
[package]
name = "my-extension"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
vsql = { path = "/path/to/vsql-rust-sdk/vsql" }
```

### 2. Write your function

In `src/lib.rs`, write a plain Rust function that takes `&[InValue]` and returns `VdfReturn`:

```rust
use vsql::{InValue, VdfReturn};

fn my_func(args: &[InValue]) -> VdfReturn {
    match args.first() {
        Some(InValue::String(s)) => VdfReturn::string(s.to_uppercase()),
        Some(InValue::Null) | None => VdfReturn::null(),
        _ => VdfReturn::error("my_func: expected a STRING argument"),
    }
}

vsql::extension! {
    funcs: [
        vsql::func!(my_func, "my_func", [vsql::Type::String] -> vsql::Type::String),
    ]
}
```

### 3. Add a manifest

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

### 4. Package, install, and test

```sh
# Build and create dist/my-extension.veb
cargo vsql package

# Package + copy to $VillageSQL_BUILD_DIR/veb_output_directory
export VillageSQL_BUILD_DIR=/path/to/villagesql/build
cargo vsql install

# Install + run the mysql-test suite
cargo vsql test
```

## The vsql_rot13 example

[`examples/vsql_rot13`](examples/vsql_rot13) is a complete, minimal extension you can use as a template:

```
examples/vsql_rot13/
├── Cargo.toml          # crate-type = ["cdylib"]
├── manifest.json       # Extension metadata
├── src/lib.rs          # ~30 lines: rot13 impl + extension! declaration
└── mysql-test/
    ├── suite.opt
    └── t/vsql_rot13.test    # SQL test script
```

To build and run its tests:

```sh
cd examples/vsql_rot13
cargo vsql test
```

To regenerate expected test results after changing behavior:

```sh
cargo vsql test --record
```

## API reference

### `Type`

SQL type for a VDF parameter or return value.

| Variant | SQL type | Rust type |
|---------|----------|-----------|
| `Type::String` | `STRING` | `&str` / `String` |
| `Type::Real` | `REAL` | `f64` |
| `Type::Int` | `INT` | `i64` |

### `InValue`

One argument delivered to your function for a single row. Always handle `Null`:

```rust
match args.first() {
    Some(InValue::String(s)) => { /* use s: &str */ }
    Some(InValue::Real(v))   => { /* use v: f64 */ }
    Some(InValue::Int(v))    => { /* use v: i64 */ }
    Some(InValue::Null) | None => VdfReturn::null(),
}
```

### `VdfReturn`

What your function returns for a single row.

| Constructor | Effect |
|-------------|--------|
| `VdfReturn::null()` | SQL NULL |
| `VdfReturn::string(s)` | String value |
| `VdfReturn::real(v)` | Floating-point value |
| `VdfReturn::int(v)` | Integer value |
| `VdfReturn::warning(msg)` | Row-level warning; NULL returned for this row, execution continues |
| `VdfReturn::error(msg)` | Fatal error; statement is aborted |

### `extension!` macro

Generates the `vef_register` / `vef_unregister` entry points that VillageSQL calls when loading your extension. List every function you want to export:

```rust
vsql::extension! {
    funcs: [
        vsql::func!(impl_fn, "sql_name", [vsql::Type::String] -> vsql::Type::String),
        vsql::func!(other_fn, "other_sql_name",
                    [vsql::Type::Int, vsql::Type::Int] -> vsql::Type::Int,
                    deterministic: true),
    ]
}
```

### `func!` macro

Builds a function descriptor and generates the C trampoline for one VDF.

```
vsql::func!(rust_fn, "sql_name", [param_types...] -> return_type)
vsql::func!(rust_fn, "sql_name", [param_types...] -> return_type, deterministic: true)
```

- `rust_fn` — your function, must be in scope
- `"sql_name"` — the name users call in SQL
- `deterministic: true` — tells the optimizer it can cache results for identical inputs (default: `false`)

## The `.veb` archive

`cargo vsql package` produces `dist/<name>.veb`, a tar archive containing:

```
manifest.json
lib/<name>.so
```

VillageSQL installs and loads this archive with:

```sql
INSTALL EXTENSION 'vsql_rot13';
-- use the function --
UNINSTALL EXTENSION 'vsql_rot13';
```

## Writing tests

Place a MySQL Test Framework suite under `mysql-test/` in your extension directory. See [`examples/vsql_rot13/mysql-test/t/vsql_rot13.test`](examples/vsql_rot13/mysql-test/t/vsql_rot13.test) for a reference test file that covers NULL handling, string operations, and table column inputs.

Run with:

```sh
cargo vsql test
```

Record expected results for new tests with:

```sh
cargo vsql test --record
```
