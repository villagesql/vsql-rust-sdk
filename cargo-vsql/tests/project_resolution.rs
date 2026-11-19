use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---- Fixture helpers -------------------------------------------------------

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cargo-vsql-project-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn extension(&self, dir: &str) -> PathBuf {
        self.write(
            &format!("{dir}/Cargo.toml"),
            "[package]\nname = \"vsql_probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
             [lib]\ncrate-type = [\"cdylib\"]\n",
        );
        self.write(
            &format!("{dir}/src/lib.rs"),
            "pub fn probe() -> u32 { 60 }\n",
        );
        self.write(
            &format!("{dir}/manifest.json"),
            "{\"name\":\"vsql_probe\",\"version\":\"0.1.0\"}",
        );
        self.0.join(dir)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        // The tooling build's target directory must not redirect fixture artifacts.
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---- End fixture helpers ---------------------------------------------------

#[test]
fn packages_from_cargos_project_root() {
    for layout in [
        "standalone",
        "plain-parent",
        "member",
        "excluded",
        "nested-workspace",
    ] {
        let project = Project::new();
        match layout {
            "plain-parent" => {
                project.write(
                    "Cargo.toml",
                    "[package]\nname = \"unrelated_parent\"\nversion = \"0.1.0\"\n",
                );
                project.write("src/lib.rs", "");
            }
            "member" => project.write(
                "Cargo.toml",
                "[workspace]\nmembers = [\"extension\"]\nresolver = \"2\"\n",
            ),
            "excluded" => project.write(
                "Cargo.toml",
                "[workspace]\nexclude = [\"extension\"]\nresolver = \"2\"\n",
            ),
            "nested-workspace" => project.write("Cargo.toml", "[workspace]\n"),
            _ => {}
        }
        let extension = project.extension("extension");
        if layout == "nested-workspace" {
            let manifest = extension.join("Cargo.toml");
            let mut content = std::fs::read_to_string(&manifest).unwrap();
            content.push_str("[workspace]\n");
            std::fs::write(manifest, content).unwrap();
        }

        // Establish Cargo validity separately so a bad fixture cannot mimic the bug.
        assert_success(&run("cargo", &["build", "--release"], &extension));
        assert_success(&run(
            env!("CARGO_BIN_EXE_cargo-vsql"),
            &["vsql", "package"],
            &extension,
        ));

        let root = if layout == "member" {
            &project.0
        } else {
            &extension
        };
        let file = std::fs::File::open(root.join("dist/vsql_probe.veb")).unwrap();
        let mut archive = tar::Archive::new(file);
        let entries: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().into_owned())
            .collect();
        assert_eq!(
            entries,
            [
                PathBuf::from("manifest.json"),
                PathBuf::from("lib/vsql_probe.so")
            ],
            "layout: {layout}"
        );
    }
}

#[test]
fn rejects_undeclared_workspace_member() {
    let project = Project::new();
    project.write("Cargo.toml", "[workspace]\nmembers = []\n");
    let extension = project.extension("extension");
    let cargo = run("cargo", &["build", "--release"], &extension);
    assert!(!cargo.status.success());
    assert!(String::from_utf8_lossy(&cargo.stderr).contains("current package believes"));
    let vsql = run(
        env!("CARGO_BIN_EXE_cargo-vsql"),
        &["vsql", "package"],
        &extension,
    );
    assert!(!vsql.status.success());
    assert!(!project.0.join("dist").exists());
    assert!(!extension.join("dist").exists());
}

#[test]
fn preserves_build_config_overrides() {
    let project = Project::new();
    project.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"extension\"]\nresolver = \"2\"\n",
    );
    let extension = project.extension("extension");
    project.write(
        "extension/src/lib.rs",
        "#[cfg(not(vsql_probe_config))]\ncompile_error!(\"config was not forwarded\");\n",
    );
    // Config file paths have historically been relative to the build's workspace root.
    project.write(
        "fixture-config.toml",
        "[build]\nrustflags = [\"--cfg\", \"vsql_probe_config\"]\n",
    );
    assert_success(&run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "vsql_probe",
            "--config",
            "fixture-config.toml",
            "--config",
            "net.offline=true",
        ],
        &project.0,
    ));
    assert_success(&run(
        env!("CARGO_BIN_EXE_cargo-vsql"),
        &[
            "vsql",
            "package",
            "--config",
            "fixture-config.toml",
            "--config",
            "net.offline=true",
        ],
        &extension,
    ));
    assert!(project.0.join("dist/vsql_probe.veb").is_file());
}
