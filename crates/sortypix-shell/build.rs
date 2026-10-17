fn main() {
    println!("cargo:rerun-if-changed=exports.def");
    // MSVC linker arguments for the COM DLL; the crate is empty on other targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rustc-link-arg-cdylib=/NOIMPLIB");
    println!("cargo:rustc-link-arg-cdylib=/NOEXP");
    let exports = std::path::PathBuf::from("exports.def")
        .canonicalize()
        .expect("COM exports");
    println!("cargo:rustc-link-arg-cdylib=/DEF:{}", exports.display());
}
