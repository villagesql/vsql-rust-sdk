# cargo-vsql

Cargo subcommand for building, packaging, and testing VillageSQL extensions.

## Installation

```sh
cargo install cargo-vsql
```

To scaffold a new extension that is preconfigured to use this CLI, use the [`vsql-extension-template-rust`](https://github.com/villagesql/vsql-extension-template-rust) cargo-generate template. For CI, reusable GitHub Actions workflows are available at [`villagesql/extension-actions`](https://github.com/villagesql/extension-actions).

## Commands

Run all commands except `inspect` from inside your extension directory (not the workspace root).

### `cargo vsql package`

Compiles the extension in release mode and produces `dist/<name>.veb` — a tar archive containing:

```
manifest.json
lib/<name>.so
```

### `cargo vsql install`

Runs `package`, then copies the `.veb` into `$VillageSQL_BUILD_DIR/lib/veb`, the directory the server reads. When `VillageSQL_BUILD_DIR` is not set, it uses `~/.villagesql/prebuilt`, where the install script puts the server.

```sh
export VillageSQL_BUILD_DIR=/path/to/villagesql/build
cargo vsql install
```

### `cargo vsql test`

Runs `install`, then executes the MySQL Test Framework suite under `mysql-test/` in your extension directory.

```sh
cargo vsql test
```

Record expected results for new or changed tests:

```sh
cargo vsql test --record
```

### `cargo vsql inspect`

Loads one or more `.veb` archives the way the server does and prints what each one registers: the VEF protocol it declares and whether the server still accepts it, the SDK version it was built against, its functions with their signatures, its custom types, its load hooks, and the preview capabilities it requires. This is the only command that does not need to run from an extension directory.

```sh
cargo vsql inspect dist/my_extension.veb
cargo vsql inspect /path/to/server/lib/veb          # every .veb in a directory
cargo vsql inspect --brief /path/to/server/lib/veb  # one line per archive
cargo vsql inspect --protocol 3 my_extension.veb    # offer an older protocol, as an older server would
```

The tool extracts `lib/<name>.so` to a temporary directory, loads it, calls its `vef_register` with a server-like argument, reads the returned registration, and calls `vef_unregister`. It never calls `on_init`, never populates a capability, and never invokes a function, but the extension's own registration code does run inside the tool's process, so only inspect archives you would be willing to install.

## The `.veb` archive

A `.veb` file is a tar archive with a flat layout:

```
manifest.json
lib/<name>.so
```

VillageSQL installs and loads it with:

```sql
INSTALL EXTENSION my_extension;
-- use the extension --
UNINSTALL EXTENSION my_extension;
```

If your server reads extensions from another directory, copy the `.veb` from `dist/` to the directory that `SHOW VARIABLES LIKE 'veb_dir'` reports.

## Writing tests

Place a MySQL Test Framework suite under `mysql-test/` in your extension directory:

```
mysql-test/
├── suite.opt
└── t/
    └── my_extension.test
```

See the [`vsql_rot13` example](https://github.com/villagesql/vsql-rust-sdk/tree/main/examples/vsql_rot13/mysql-test/t/vsql_rot13.test) for a reference test file covering NULL handling, string operations, and table column inputs.
