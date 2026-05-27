# Contributing to the VillageSQL Rust SDK

## Repository layout

| Path | Purpose |
|------|---------|
| `vsql-sys/` | Raw FFI bindings (`bindings.rs` + generated entry points) |
| `vsql/` | Safe Rust wrapper — this is what extension authors use |
| `cargo-vsql/` | The `cargo vsql` subcommand |
| `examples/` | Reference extensions |
| `tests/` | SDK integration tests |
| `include/` | VillageSQL C++ ABI headers |

## Development setup

Normal builds and tests require only a stable Rust toolchain — no system libraries.

```sh
cargo test
```

## Updating the ABI bindings

`vsql-sys/src/bindings.rs` is a pre-generated file committed to the repository. It is produced from `include/villagesql/abi/types.h` via [bindgen](https://github.com/rust-lang/rust-bindgen) and must be regenerated whenever that header changes.

Regeneration requires `libclang-dev` (Ubuntu/Debian) or the equivalent LLVM development package for your platform.

### Steps

1. Install `libclang-dev`:

   ```sh
   # Ubuntu / Debian
   sudo apt-get install -y libclang-dev

   # macOS (via Homebrew)
   brew install llvm
   ```

2. Regenerate the bindings:

   ```sh
   cargo build -p vsql-sys --features vsql-sys/regenerate-bindings
   ```

   This overwrites `vsql-sys/src/bindings.rs` in place.

3. Verify the result compiles cleanly:

   ```sh
   cargo test
   ```

4. Commit `vsql-sys/src/bindings.rs` together with the header change.

### CI enforcement

The `check-bindings` CI job regenerates `bindings.rs` and runs `git diff --exit-code` against it. Any PR that modifies `include/villagesql/abi/types.h` without updating `bindings.rs` will fail this check.

## Release checklist

- [ ] Update version in `vsql-sys/Cargo.toml`, `vsql/Cargo.toml`, and `cargo-vsql/Cargo.toml`
- [ ] Regenerate bindings if headers changed (see above)
- [ ] `cargo test` passes
- [ ] CI is green
