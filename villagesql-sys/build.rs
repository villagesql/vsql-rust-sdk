fn main() {
    println!("cargo:rerun-if-changed=../include/villagesql/abi/types.h");
    println!("cargo:rerun-if-changed=../include/villagesql/abi/preview/ping.h");
    println!("cargo:rerun-if-changed=../include/villagesql/abi/preview/sys_var.h");
    println!("cargo:rerun-if-changed=../include/villagesql/abi/preview/status_var.h");
    println!("cargo:rerun-if-changed=../include/villagesql/abi/preview/keyring.h");

    #[cfg(feature = "regenerate-bindings")]
    regenerate();
}

#[cfg(feature = "regenerate-bindings")]
fn regenerate() {
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.parent().unwrap().join("include");
    let out_dir = manifest_dir.join("src/bindings");
    std::fs::create_dir_all(&out_dir).expect("create bindings dir");

    // (header path relative to include/, output file name)
    let headers = [
        ("villagesql/abi/types.h", "types.rs"),
        ("villagesql/abi/preview/ping.h", "ping.rs"),
        ("villagesql/abi/preview/sys_var.h", "sys_var.rs"),
        ("villagesql/abi/preview/status_var.h", "status_var.rs"),
        ("villagesql/abi/preview/keyring.h", "keyring.rs"),
    ];

    for (header, out_file) in headers {
        let bindings = bindgen::Builder::default()
            .header(include_dir.join(header).to_str().unwrap())
            .clang_arg(format!("-I{}", include_dir.display()))
            .clang_arg("-x")
            .clang_arg("c++")
            .clang_arg("-std=c++17")
            .allowlist_type("vef_.*")
            .allowlist_type("VEF_.*")
            .allowlist_var("VEF_.*")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");

        bindings
            .write_to_file(out_dir.join(out_file))
            .expect("Couldn't write bindings");
    }
}
