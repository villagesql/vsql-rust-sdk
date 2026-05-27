# Contributing to the VillageSQL Rust SDK

## Repository layout

| Path | Purpose |
|------|---------|
| `villagesql-sys/` | Raw FFI bindings (`bindings.rs` + generated entry points) |
| `villagesql/` | `villagesql` crate — safe Rust wrapper for extension authors |
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

`villagesql-sys/src/bindings.rs` is a pre-generated file committed to the repository. It is produced from `include/villagesql/abi/types.h` via [bindgen](https://github.com/rust-lang/rust-bindgen) and must be regenerated whenever that header changes.

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
   cargo build -p villagesql-sys --features villagesql-sys/regenerate-bindings
   ```

   This overwrites `villagesql-sys/src/bindings.rs` in place.

3. Verify the result compiles cleanly:

   ```sh
   cargo test
   ```

4. Commit `villagesql-sys/src/bindings.rs` together with the header change.

### CI enforcement

The `check-bindings` CI job regenerates `bindings.rs` and runs `git diff --exit-code` against it. Any PR that modifies `include/villagesql/abi/types.h` without updating `bindings.rs` will fail this check.

## Release checklist

- [ ] Update version in `villagesql-sys/Cargo.toml` (`villagesql-sys`), `villagesql/Cargo.toml` (`villagesql`), and `cargo-vsql/Cargo.toml`
- [ ] Regenerate bindings if headers changed (see above)
- [ ] `cargo test` passes
- [ ] CI is green
- [ ] Publish to crates.io (see below)

## Publishing to crates.io

Crates must be published in dependency order: `villagesql-sys` first, then `villagesql`, then `cargo-vsql`.

1. Log in (one-time):

   ```sh
   cargo login
   ```

2. Publish each crate in order:

   ```sh
   cargo publish -p villagesql-sys
   cargo publish -p villagesql
   cargo publish -p cargo-vsql
   ```

   There may be a short delay (~30s) between a crate becoming available on crates.io and the registry index propagating it. If a publish step fails with a "not found" error for a dependency, wait a moment and retry.

To yank a version (prevents new projects from resolving it, but does not delete it):

```sh
cargo yank villagesql-sys@x.y.z
cargo yank villagesql@x.y.z
cargo yank cargo-vsql@x.y.z
```

Note: yanked version numbers cannot be reused. If you need to republish, use the next patch version.
