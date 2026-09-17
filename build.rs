// Embeds app.manifest into the executable: DPI awareness (PerMonitorV2),
// long-path support, UTF-8 code page, and Common Controls v6 theming.
// Also compiles app.rc so the app icon ships inside the exe.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/jamb.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = match stamp_version(&manifest_dir) {
        Ok(p) => p,
        // Shipping a manifest that says the wrong version is worse than not
        // building: the number is what a bug report will quote back at you.
        Err(e) => panic!("stamping the manifest version failed: {e}"),
    };
    // MSVC linker: /MANIFEST:EMBED consumes the input manifest.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );

    if let Err(e) = embed_icon(&manifest_dir) {
        // A missing resource compiler must not silently ship a blank icon: fail
        // the build so the gap is obvious.
        panic!("embedding the app icon failed: {e}");
    }
}

/// Copy `app.manifest` into OUT_DIR with its version taken from Cargo.toml.
///
/// The assembly identity wants four parts where a crate version has three, so
/// the fourth is always zero. Doing it here means the version lives in exactly
/// one file — a manifest still saying 0.2.0 inside a 0.3.0 build is the kind of
/// drift nobody notices until somebody quotes it back in a bug report.
fn stamp_version(manifest_dir: &Path) -> Result<PathBuf, String> {
    let src = manifest_dir.join("app.manifest");
    let text = std::fs::read_to_string(&src).map_err(|e| format!("reading app.manifest: {e}"))?;
    let version = format!(
        "{}.0",
        std::env::var("CARGO_PKG_VERSION").map_err(|e| e.to_string())?
    );

    // The first win32 identity and no other. A manifest names the assembly
    // itself before it names anything it depends on, so the first one is ours;
    // the Common Controls dependency below it is an `assemblyIdentity
    // type="win32"` too, and today it is only spared because its version sits
    // on the following line.
    let mut found = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let identity = (!found)
            .then(|| line.find(r#"<assemblyIdentity type="win32""#))
            .flatten();
        match identity.and(attr_span(line)) {
            Some((start, end)) => {
                found = true;
                out.push_str(&line[..start]);
                out.push_str(&version);
                out.push_str(&line[end..]);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    if !found {
        return Err("no win32 assemblyIdentity with a version= attribute".into());
    }

    let dest = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?)
        .join("app.manifest");
    std::fs::write(&dest, out).map_err(|e| format!("writing {}: {e}", dest.display()))?;
    Ok(dest)
}

/// The bounds of the value inside the first `version="..."` on a line.
fn attr_span(line: &str) -> Option<(usize, usize)> {
    let start = line.find(r#"version=""#)? + r#"version=""#.len();
    let end = start + line[start..].find('"')?;
    Some((start, end))
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
