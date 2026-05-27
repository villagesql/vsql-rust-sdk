# VillageSQL Rust SDK

Write custom SQL functions (VDFs) for VillageSQL in safe Rust. The SDK handles all FFI marshaling so you work entirely in ordinary Rust types.

## Prerequisites

- [Rust toolchain](https://rustup.rs) (stable)
- `cargo-vsql` installed (see [Installing cargo-vsql](#installing-cargo-vsql))
- VillageSQL build directory (for `install` and `test` commands)

## Installing cargo-vsql

Clone the repository and install the CLI tool:

```sh
git clone https://github.com/villagesql/vsql-rust-sdk.git
cd vsql-rust-sdk
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
[workspace]
members = ["."]
resolver = "2"

[package]
name = "my-extension"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
vsql = { path = "/path/to/vsql-rust-sdk/vsql" }  # replace with your clone path
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
export VillageSQL_BUILD_DIR=/path/to/villagesql/build
cd examples/vsql_rot13
cargo vsql test
```

To regenerate expected test results after changing behavior:

```sh
cargo vsql test --record
```

## The vsql_rational example

[`examples/vsql_rational`](examples/vsql_rational) is a complete custom-type extension implementing an exact rational number type (`n/d`). It demonstrates:

- `custom_type!` with `encode`, `decode`, `compare`, `hash`, and `default`
- Additional VDFs operating on the custom type (`rational_add`, `rational_sub`, `rational_mul`, `rational_div`, `rational_numer`, `rational_denom`, `rational_to_real`)
- Using `vsql::custom!("rational")` in `func!` signatures
- Receiving custom-type arguments via `InValue::Custom(&[u8])` and returning results via `VdfReturn::binary(Vec<u8>)`

```
examples/vsql_rational/
├── Cargo.toml
├── manifest.json
├── src/lib.rs        # type impl + arithmetic VDFs + extension! declaration
└── mysql-test/
    └── t/vsql_rational.test
```

## API reference

### `Type`

SQL type for a VDF parameter or return value.

| Variant | SQL type | Rust type |
|---------|----------|-----------|
| `Type::String` | `STRING` | `&str` / `String` |
| `Type::Real` | `REAL` | `f64` |
| `Type::Int` | `INT` | `i64` |
| `Type::Custom(name)` | custom type | `&[u8]` (persisted binary) |

Use the [`custom!`](#custom-macro) macro to construct `Type::Custom` — never build it by hand.

### `InValue`

One argument delivered to your function for a single row. Always handle `Null`:

```rust
match args.first() {
    Some(InValue::String(s))  => { /* use s: &str */ }
    Some(InValue::Real(v))    => { /* use v: f64 */ }
    Some(InValue::Int(v))     => { /* use v: i64 */ }
    Some(InValue::Custom(b))  => { /* use b: &[u8] — persisted binary bytes */ }
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
| `VdfReturn::binary(v)` | Custom-type value — `v: Vec<u8>` in persisted binary format |
| `VdfReturn::warning(msg)` | Row-level warning; NULL returned for this row, execution continues |
| `VdfReturn::error(msg)` | Fatal error; statement is aborted |

### `extension!` macro

Generates the `vef_register` / `vef_unregister` entry points that VillageSQL calls when loading your extension. List every function and custom type you want to export:

```rust
vsql::extension! {
    funcs: [
        vsql::func!(impl_fn, "sql_name", [vsql::Type::String] -> vsql::Type::String),
        vsql::func!(other_fn, "other_sql_name",
                    [vsql::Type::Int, vsql::Type::Int] -> vsql::Type::Int,
                    deterministic: true),
    ],
    // Optional — omit entirely if you have no custom types.
    types: [
        vsql::custom_type!(
            type_name: "my_type",
            persisted_length: 16,
            max_decode_buffer_length: 64,
            encode: my_encode,
            decode: my_decode,
            compare: my_compare,
        ),
    ]
}
```

The `types:` list is optional; omitting it is equivalent to `types: []`.

### `custom!` macro

Produces a `Type::Custom` value for use in `func!` signatures. Because `Type::Custom` must hold a null-terminated static C string, always use this macro rather than constructing the variant directly:

```rust
vsql::custom!("rational")   // → Type::Custom pointing to b"rational\0"
```

### `custom_type!` macro

Registers a custom SQL type and automatically generates its four required SQL-callable VDFs (`TYPE::from_string`, `TYPE::to_string`, `TYPE::compare`, and optionally `TYPE::hash`).

```rust
vsql::custom_type!(
    type_name: "rational",          // SQL type name
    persisted_length: 16,           // fixed byte size in storage
    max_decode_buffer_length: 42,   // upper bound on string representation length
    encode: rational_encode,        // fn(&str) -> Result<Vec<u8>, String>
    decode: rational_decode,        // fn(&[u8]) -> Result<String, String>
    compare: rational_compare,      // fn(&[u8], &[u8]) -> std::cmp::Ordering
    hash: rational_hash,            // fn(&[u8]) -> usize  (optional, enables hashed indexes)
    default: "0/1",                 // optional intrinsic default, encoded at install time
)
```

Required fields: `type_name`, `persisted_length`, `max_decode_buffer_length`, `encode`, `decode`, `compare`.

Optional fields: `hash` (recommended whenever the type will be used in indexes or hash joins), `default`.

The four generated VDFs are also exported as callable SQL functions:

| Generated VDF | Signature |
|---|---|
| `TYPE::from_string` | `(STRING) -> CUSTOM` |
| `TYPE::to_string` | `(CUSTOM) -> STRING` |
| `TYPE::compare` | `(CUSTOM, CUSTOM) -> INT` |
| `TYPE::hash` | `(CUSTOM) -> INT` — only if `hash:` is provided |

VDFs that operate on your custom type use `vsql::custom!("type_name")` in their `func!` signature and receive / return the persisted binary via `InValue::Custom(&[u8])` and `VdfReturn::binary(Vec<u8>)`.

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

`cargo vsql install` copies the VEB into `$VillageSQL_BUILD_DIR/veb_output_directory`, which is where the MTR test server reads from. If you're running a standalone installed server, copy the VEB from `dist/` to that server's configured `veb_dir` instead.

VillageSQL installs and loads this archive with:

```sql
INSTALL EXTENSION vsql_rot13;
-- use the function --
UNINSTALL EXTENSION vsql_rot13;
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
