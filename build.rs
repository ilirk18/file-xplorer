// Embeds app.manifest into the executable: DPI awareness (PerMonitorV2),
// long-path support, UTF-8 code page, and Common Controls v6 theming.
// Also compiles app.rc so the sash icon ships inside the exe.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = manifest_dir.join("app.manifest");
    // MSVC linker: /MANIFEST:EMBED consumes the input manifest.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );

    if let Err(e) = embed_icon(&manifest_dir) {
        // A missing resource compiler must not silently ship a blank icon: fail
        // the build so the gap is obvious.
        panic!("embedding app.ico failed: {e}");
    }
}

fn embed_icon(manifest_dir: &Path) -> Result<(), String> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let res = out_dir.join("app.res");
    let rc = find_rc().ok_or_else(|| "rc.exe not found (Windows SDK)".to_string())?;

    let status = Command::new(&rc)
        .args([
            "/nologo",
            "/fo",
            res.to_str().ok_or("res path")?,
            manifest_dir.join("app.rc").to_str().ok_or("rc path")?,
        ])
        .current_dir(manifest_dir)
        .status()
        .map_err(|e| format!("running rc.exe: {e}"))?;
    if !status.success() {
        return Err(format!("rc.exe exited with {status}"));
    }

    println!("cargo:rustc-link-arg-bins={}", res.display());
    Ok(())
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = which("rc.exe") {
        return Some(p);
    }
    let roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];
    for root in roots {
        let root = Path::new(root);
        if !root.is_dir() {
            continue;
        }
        let mut versions: Vec<PathBuf> = std::fs::read_dir(root)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        for ver in versions.into_iter().rev() {
            let candidate = ver.join(r"x64\rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(())
}
