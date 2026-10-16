//! A harmless executable substituted for Sortypix only inside disposable shell probes.
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(), 1, "Exactly one folder argument");
    let output = std::env::var_os("SORTYPIX_SHELL_PROBE_OUTPUT").expect("Probe output");
    std::fs::write(output, args[0].as_encoded_bytes()).expect("Record received folder");
}
