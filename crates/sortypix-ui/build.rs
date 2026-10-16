fn main() {
    println!("cargo:rerun-if-changed=../../assets/sortypix.ico");
    let out =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let icon = std::path::PathBuf::from("../../assets/sortypix.ico")
        .canonicalize()
        .expect("Application icon");
    let source = out.join("sortypix.rc");
    let resource = out.join("sortypix.res");
    std::fs::write(
        &source,
        format!(
            "1 ICON \"{}\"\n",
            icon.display().to_string().replace('\\', "/")
        ),
    )
    .expect("Resource script");
    let status = std::process::Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&source)
        .status()
        .expect("Windows SDK resource compiler");
    assert!(status.success(), "Resource compilation failed");
    println!("cargo:rustc-link-arg-bin=sortypix={}", resource.display());
}
