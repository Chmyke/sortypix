fn main() {
    println!("cargo:rerun-if-changed=../../assets/sortypix.ico");
    // The executable icon is a Windows resource; Linux reads it from the desktop entry.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
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
    // Sortypix AI shows the same icon (its binary exists only with the ai feature).
    if std::env::var_os("CARGO_FEATURE_AI").is_some() {
        println!(
            "cargo:rustc-link-arg-bin=sortypix-ai={}",
            resource.display()
        );
    }
}
