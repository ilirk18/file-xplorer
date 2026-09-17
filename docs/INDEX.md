# File Xplorer – Documentation Index

Start here. Each phase has its own doc; this index links them all and summarises
the current state so context is never lost.

---

## Quick reference

| Phase | Doc | Status | Summary |
|-------|-----|--------|---------|
| 1 | [phase-1.md](phase-1.md) | Done | Win32 window, Direct2D, message loop |
| 2 | [phase-2.md](phase-2.md) | Done | Directory listing, virtual list, scrollbar |
| 3 | [phase-3.md](phase-3.md) | Done | Dual pane, tabs, breadcrumbs, keyboard nav |
| 4 | [phase-4.md](phase-4.md) | Done | Shell file operations, multi-select, async I/O, DPI |
| 5 | [phase-5.md](phase-5.md) | Done | Layout module, sidebar, filter, theming, visual overhaul |
| 6 | *(not started)* | Planned | Command palette, fuzzy search, drag & drop, settings persistence, installer |

---

## Current codebase

| Module | Responsibility |
|---|---|
| `src/main.rs` | Window creation, `wndproc`, input handling, worker-thread plumbing, modal prompts, context menu |
| `src/layout.rs` | Every rectangle in the UI, plus `hit_test`. No Win32 types, fully unit-tested |
| `src/renderer.rs` | Direct2D/DirectWrite drawing, WIC icon bitmaps, `TextMeasurer` |
| `src/theme.rs` | `Palette` tokens for dark and light, with contrast tests |
| `src/pane.rs` | `Tab` (listing, history, scroll), `Pane`, `LoadRequest`/`finish_load` generation guarding |
| `src/file_list.rs` | Virtual list, multi-select, sort (folders first, natural order), live filter |
| `src/fs.rs` | `list_dir`, path helpers, `validate_file_name`, `drives()`, size/date formatting |
| `src/ops.rs` | `IFileOperation` for copy/move/delete/rename/new-folder, `CF_HDROP` clipboard, `ShellExecuteW` |
| `src/icons.rs` | `HICON` cache keyed by extension or path, handles destroyed on drop |
| `src/config.rs` | `key=value` settings file: pane count, theme, sidebar, hidden files, split |
| `build.rs` + `app.manifest` | Embeds the manifest: PerMonitorV2 DPI, long paths, UTF-8, Common Controls v6 |

### Architectural rules

1. **Geometry lives in one place.** If something is drawn, `layout.rs` positions
   it and `hit_test` resolves clicks against the same rectangle. Never compute a
   rectangle in `renderer.rs` or `main.rs`.
2. **The UI thread never touches the disk.** Directory reads and file operations
   are posted to worker threads and return via `WM_APP_*` messages. Results carry
   a generation number and stale ones are dropped.
3. **Destructive work goes through the shell.** `ops.rs` uses `IFileOperation`
   so the Recycle Bin, progress, conflict resolution, elevation and undo are the
   system's, not ours.
4. **Errors are surfaced.** No `let _ = ` on anything a person needs to know about.
5. **Sizes are physical pixels** derived from `Metrics::for_dpi`. Nothing assumes 96 DPI.

---

## Conventions

- **Tests**: after each phase add tests; before the next phase run them and fix
  failures. See [.cursor/rules/phase-tests.mdc](../.cursor/rules/phase-tests.mdc).
- **Docs**: after each phase add or update `docs/phase-N.md` and this index.

Current: **102 tests, zero warnings** (`cargo test`).

---

## Running

- **Build**: `cargo build --release`
- **Run**: `cargo run --release`
- Requires Windows 10 1703+ and a Rust toolchain.

---

*Last updated: after Phase 5 (interface overhaul).*
