fn main() {
    println!("cargo:rerun-if-changed=exports.def");
    println!("cargo:rustc-link-arg-cdylib=/NOIMPLIB");
    println!("cargo:rustc-link-arg-cdylib=/NOEXP");
    let exports = std::path::PathBuf::from("exports.def")
        .canonicalize()
        .expect("COM exports");
    println!("cargo:rustc-link-arg-cdylib=/DEF:{}", exports.display());
}
