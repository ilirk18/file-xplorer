// Embeds app.manifest into the executable: DPI awareness (PerMonitorV2),
// long-path support, UTF-8 code page, and Common Controls v6 theming.
// Doing it via the manifest rather than at runtime means the process is
// DPI-aware before the first window is created, so nothing is ever bitmap-stretched.

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
        // MSVC linker: /MANIFEST:EMBED consumes the input manifest.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
