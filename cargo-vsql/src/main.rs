use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;

// Cargo dispatches `cargo vsql` to the `cargo-vsql` binary as:
//   cargo-vsql vsql <subcommand> [args]
// The outer `Cargo` enum absorbs the `vsql` token.
#[derive(Parser)]
#[command(name = "cargo-vsql", bin_name = "cargo")]
enum Cargo {
    Vsql(Args),
}

#[derive(clap::Args)]
#[command(about = "VillageSQL extension tooling")]
struct Args {
    /// Forward a `--config KEY=VALUE` override to `cargo build` (repeatable).
    /// Handy for patching deps during testing, e.g.
    /// --config 'patch.crates-io.villagesql.path="/path/to/villagesql"'
    #[arg(long, global = true, value_name = "KEY=VALUE")]
    config: Vec<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build and package the extension in the current directory as a .veb archive
    Package,
    /// Package and install to `$VillageSQL_BUILD_DIR/lib/veb`, the directory the server reads
    Install,
    /// Install and run the mysql-test suite
    Test {
        /// Regenerate expected .result files
        #[arg(long)]
        record: bool,
    },
}

#[derive(Deserialize)]
struct CargoManifest {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
}

fn main() -> Result<()> {
    let Cargo::Vsql(args) = Cargo::parse();

    let ext_dir = std::env::current_dir()?;
    let ext_name = extension_name(&ext_dir)?;
    let workspace_root = workspace_root(&ext_dir)?;

    match args.cmd {
        Cmd::Package => {
            let veb = package(&ext_name, &ext_dir, &workspace_root, &args.config)?;
            println!("Created: {}", veb.display());
        }
        Cmd::Install => {
            let veb = package(&ext_name, &ext_dir, &workspace_root, &args.config)?;
            install(&ext_name, &veb)?;
        }
        Cmd::Test { record } => {
            let veb = package(&ext_name, &ext_dir, &workspace_root, &args.config)?;
            install(&ext_name, &veb)?;
            run_tests(&ext_dir, record)?;
        }
    }

    Ok(())
}

// ---- Discovery helpers ----------------------------------------------------

fn extension_name(dir: &Path) -> Result<String> {
    let path = dir.join("Cargo.toml");
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let raw: toml::Value =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    if raw.get("workspace").is_some() && raw.get("package").is_none() {
        bail!(
            "{} is a workspace root, not an extension — run `cargo vsql` from within an extension directory (e.g. examples/vsql_rot13)",
            path.display()
        );
    }
    let manifest: CargoManifest =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    Ok(manifest.package.name)
}

/// Let Cargo distinguish workspace members, excluded packages, and standalone projects.
fn workspace_root(start: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .current_dir(start)
        .output()
        .context("running cargo locate-project")?;
    if !output.status.success() {
        bail!(
            "cargo locate-project failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let manifest = String::from_utf8(output.stdout)
        .context("cargo locate-project returned a non-UTF-8 manifest path")?;
    Path::new(manifest.trim())
        .parent()
        .map(Path::to_path_buf)
        .context("cargo locate-project returned no manifest directory")
}

// TODO(villagesql-windows): handle dlls
fn find_lib(name: &str, workspace_root: &Path) -> Result<PathBuf> {
    let release_dir = workspace_root.join("target").join("release");
    let lib_name = name.replace('-', "_");
    for ext in ["so", "dylib"] {
        let candidate = release_dir.join(format!("lib{lib_name}.{ext}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "compiled library for '{}' not found in {} — run `cargo build --release -p {}` first",
        name,
        release_dir.display(),
        name
    )
}

// ---- End discovery helpers ------------------------------------------------

// ── Subcommand implementations ────────────────────────────────────────────────
// TODO(villagesql): handle building extension in debug mode.
fn package(
    name: &str,
    ext_dir: &Path,
    workspace_root: &Path,
    config: &[String],
) -> Result<PathBuf> {
    // Build the extension.
    println!("Building {name}...");
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "-p", name]);
    // Forward any `--config KEY=VALUE` overrides (e.g. patching deps to a local path).
    for cfg in config {
        cmd.args(["--config", cfg]);
    }
    let status = cmd
        .current_dir(workspace_root)
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let lib = find_lib(name, workspace_root)?;

    let manifest_json = ext_dir.join("manifest.json");
    if !manifest_json.exists() {
        bail!("manifest.json not found in {}", ext_dir.display());
    }

    // Build the .veb tar archive.
    let dist = workspace_root.join("dist");
    std::fs::create_dir_all(&dist)?;
    let veb_path = dist.join(format!("{name}.veb"));

    let file = std::fs::File::create(&veb_path)
        .with_context(|| format!("creating {}", veb_path.display()))?;
    let mut archive = tar::Builder::new(file);
    archive.append_path_with_name(&manifest_json, "manifest.json")?;
    // Always use .so regardless of platform (server expects .so).
    archive.append_path_with_name(&lib, format!("lib/{name}.so"))?;
    archive.finish()?;

    Ok(veb_path)
}

fn build_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("VillageSQL_BUILD_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    let prebuilt = PathBuf::from(&home).join(".villagesql").join("prebuilt");
    if prebuilt.exists() {
        return Ok(prebuilt);
    }
    Ok(PathBuf::from(&home).join(".villagesql"))
}

// The server reads `lib/veb`. A source build tree has it as a link to
// `veb_output_directory`; a release tarball has only `lib/veb`.
fn veb_install_dir(build_dir: &Path) -> PathBuf {
    let lib_veb = build_dir.join("lib").join("veb");
    if lib_veb.is_dir() {
        lib_veb
    } else {
        build_dir.join("veb_output_directory")
    }
}

fn install(name: &str, veb: &Path) -> Result<()> {
    let install_dir = veb_install_dir(&build_dir()?);
    std::fs::create_dir_all(&install_dir)?;
    let dest = install_dir.join(format!("{name}.veb"));
    std::fs::copy(veb, &dest)?;
    println!("Installed to: {}", dest.display());
    Ok(())
}

fn run_tests(ext_dir: &Path, record: bool) -> Result<()> {
    let mtr = build_dir()?.join("mysql-test").join("mysql-test-run.pl");
    if !mtr.exists() {
        bail!("MTR not found at {}", mtr.display());
    }

    let suite_dir = ext_dir.join("mysql-test");
    if !suite_dir.exists() {
        bail!("no mysql-test/ directory in {}", ext_dir.display());
    }

    let mut cmd = Command::new("perl");
    cmd.arg(&mtr)
        .arg(format!("--suite={}", suite_dir.display()))
        .current_dir(mtr.parent().unwrap());
    if record {
        cmd.arg("--record");
    }

    let status = cmd.status().context("running mysql-test-run.pl")?;
    if !status.success() {
        bail!("tests failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::veb_install_dir;

    #[test]
    fn installs_into_lib_veb_when_present() {
        let dir = std::env::temp_dir().join(format!("cargo-vsql-libveb-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib").join("veb")).unwrap();
        assert_eq!(veb_install_dir(&dir), dir.join("lib").join("veb"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn falls_back_to_veb_output_directory() {
        let dir = std::env::temp_dir().join(format!("cargo-vsql-noveb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(veb_install_dir(&dir), dir.join("veb_output_directory"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
