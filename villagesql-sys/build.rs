fn main() {
    println!("cargo:rerun-if-changed=../include/villagesql/abi/types.h");
    println!("cargo:rerun-if-changed=../include/villagesql/abi/preview/storage.h");

    #[cfg(feature = "regenerate-bindings")]
    regenerate();
}

#[cfg(feature = "regenerate-bindings")]
fn regenerate() {
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.parent().unwrap().join("include");
    let types_h = include_dir.join("villagesql/abi/types.h");

    let bindings = bindgen::Builder::default()
        .header(types_h.to_str().unwrap())
        .clang_arg(format!("-I{}", include_dir.display()))
        // types.h uses C++ typed enums (enum : unsigned int)
        .clang_arg("-x")
        .clang_arg("c++")
        .clang_arg("-std=c++17")
        // Only generate type/constant bindings; the storage.h free functions
        // are server-side symbols resolved at dlopen time, not link time.
        .allowlist_type("vef_.*")
        .allowlist_type("VEF_.*")
        .allowlist_var("VEF_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(manifest_dir.join("src/bindings.rs"))
        .expect("Couldn't write bindings");
}
