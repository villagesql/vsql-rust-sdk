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
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build and package the extension in the current directory as a .veb archive
    Package,
    /// Package and install to $VillageSQL_BUILD_DIR/veb_output_directory
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
            let veb = package(&ext_name, &ext_dir, &workspace_root)?;
            println!("Created: {}", veb.display());
        }
        Cmd::Install => {
            let veb = package(&ext_name, &ext_dir, &workspace_root)?;
            install(&ext_name, &veb)?;
        }
        Cmd::Test { record } => {
            let veb = package(&ext_name, &ext_dir, &workspace_root)?;
            install(&ext_name, &veb)?;
            run_tests(&ext_dir, record)?;
        }
    }

    Ok(())
}

// ── Discovery ─────────────────────────────────────────────────────────────────

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

fn workspace_root(start: &Path) -> Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = std::fs::read_to_string(&cargo_toml)?;
            let parsed: toml::Value = toml::from_str(&content)?;
            if parsed.get("workspace").is_some() {
                return Ok(dir);
            }
        }
        if !dir.pop() {
            bail!(
                "could not find workspace root (no Cargo.toml with [workspace] above {})",
                start.display()
            );
        }
    }
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

// ── Subcommand implementations ────────────────────────────────────────────────
// TODO(villagesql): handle building extension in debug mode.
fn package(name: &str, ext_dir: &Path, workspace_root: &Path) -> Result<PathBuf> {
    // Build the extension.
    println!("Building {name}...");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", name])
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

fn install(name: &str, veb: &Path) -> Result<()> {
    let build_dir =
        std::env::var("VillageSQL_BUILD_DIR").context("VillageSQL_BUILD_DIR is not set")?;
    let install_dir = PathBuf::from(build_dir).join("veb_output_directory");
    std::fs::create_dir_all(&install_dir)?;
    let dest = install_dir.join(format!("{name}.veb"));
    std::fs::copy(veb, &dest)?;
    println!("Installed to: {}", dest.display());
    Ok(())
}

fn run_tests(ext_dir: &Path, record: bool) -> Result<()> {
    let build_dir =
        std::env::var("VillageSQL_BUILD_DIR").context("VillageSQL_BUILD_DIR is not set")?;

    let mtr = PathBuf::from(&build_dir)
        .join("mysql-test")
        .join("mysql-test-run.pl");
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
