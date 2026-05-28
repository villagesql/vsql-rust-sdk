# cargo-vsql

Cargo subcommand for building, packaging, and testing VillageSQL extensions.

## Installation

```sh
cargo install cargo-vsql
```

To scaffold a new extension that is preconfigured to use this CLI, use the [`vsql-extension-template-rust`](https://github.com/villagesql/vsql-extension-template-rust) cargo-generate template. For CI, reusable GitHub Actions workflows are available at [`villagesql/extension-actions`](https://github.com/villagesql/extension-actions).

## Commands

Run all commands from inside your extension directory (not the workspace root).

### `cargo vsql package`

Compiles the extension in release mode and produces `dist/<name>.veb` — a tar archive containing:

```
manifest.json
lib/<name>.so
```

### `cargo vsql install`

Runs `package`, then copies the `.veb` into `$VillageSQL_BUILD_DIR/veb_output_directory`. Required before running tests against a local VillageSQL build.

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

If you're running a standalone installed server (not an MTR test server), copy the `.veb` from `dist/` to the server's configured `veb_dir` instead of using `cargo vsql install`.

## Writing tests

Place a MySQL Test Framework suite under `mysql-test/` in your extension directory:

```
mysql-test/
├── suite.opt
└── t/
    └── my_extension.test
```

See the [`vsql_rot13` example](https://github.com/villagesql/vsql-rust-sdk/tree/main/examples/vsql_rot13/mysql-test/t/vsql_rot13.test) for a reference test file covering NULL handling, string operations, and table column inputs.
