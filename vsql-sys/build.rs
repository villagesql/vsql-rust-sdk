use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.parent().unwrap().join("include");
    let types_h = include_dir.join("villagesql/abi/types.h");
    let storage_h = include_dir.join("villagesql/abi/preview/storage.h");

    println!("cargo:rerun-if-changed={}", types_h.display());
    println!("cargo:rerun-if-changed={}", storage_h.display());

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

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings");
}
