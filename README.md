# File Xplorer

Minimal native dual-pane file manager for Windows. Phase 1 (scaffold + window) and Phase 2 (file listing) are done.

**Full context and phase docs:** see **[docs/INDEX.md](docs/INDEX.md)**. It links to [phase-1.md](docs/phase-1.md) and [phase-2.md](docs/phase-2.md) so you never lose context.

---

## Requirements

- **Rust** (e.g. [rustup](https://rustup.rs/))
- **Windows 10+** (Direct2D, Win32)

---

## Cargo not found

If you see `cargo : The term 'cargo' is not recognized`:

1. **Install Rust** (if needed): run the installer from [rustup.rs](https://rustup.rs/) and follow the prompts.
2. **Use a new terminal** after installation so PATH is updated (or restart the terminal/IDE).
3. **Or add Cargo to PATH manually** (PowerShell, current user):
   ```powershell
   $cargo = "$env:USERPROFILE\.cargo\bin"
   if (Test-Path $cargo) { $env:Path = "$cargo;$env:Path" }
   cargo run --release
   ```
4. **Or run via full path** (adjust if you installed elsewhere):
   ```powershell
   & "$env:USERPROFILE\.cargo\bin\cargo.exe" run --release
   ```

---

## Build

```bash
cargo build --release
```

The executable is at `target/release/file-xplorer.exe`.

---

## Run

```bash
cargo run --release
```

A single window opens showing the current directory (or `C:\`) as a scrollable file list. Use the scrollbar or mouse wheel. Resize and close as normal.

---

## Size check (optional)

Target for the full project is &lt; 3 MB.

```powershell
(Get-Item target\release\file-xplorer.exe).Length / 1MB
```

For more detail: [cargo-bloat](https://github.com/RazrFalcon/cargo-bloat).

---

## Documentation

| Doc | Purpose |
|-----|--------|
| **[docs/INDEX.md](docs/INDEX.md)** | **Start here.** Index of all phases, current codebase summary, conventions. |
| [docs/phase-1.md](docs/phase-1.md) | Phase 1: scaffold, Win32 window, Direct2D. |
| [docs/phase-2.md](docs/phase-2.md) | Phase 2: file listing, virtual list, scrollbar. |

After each new phase, update the index and add `docs/phase-N.md` so context is never lost.

---

## Project layout

- `src/main.rs` – entry point, Win32 window, message loop, scroll/paint
- `src/renderer.rs` – Direct2D, DirectWrite, file list drawing
- `src/pane.rs` – current path, file list, load_path
- `src/file_list.rs` – virtual list, scroll, sort
- `src/fs.rs` – FileEntry, list_dir, current_directory
- `src/icons.rs` – IconCache (Phase 2: cache only)
- `src/search.rs`, `src/palette.rs`, `src/config.rs`, `src/theme.rs` – stubs for later phases
