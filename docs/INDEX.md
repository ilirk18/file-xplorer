# File Xplorer – Documentation Index

Use this index to get full context at any time. Each phase has its own doc; the index links to all of them and summarizes the current state so context is never lost.

---

## Quick reference

| Phase | Doc | Status | Summary |
|-------|-----|--------|---------|
| 1 | [phase-1.md](phase-1.md) | Done | Win32 window, Direct2D, message loop, clear background |
| 2 | [phase-2.md](phase-2.md) | Done | Directory listing, virtual list, scrollbar, file list rendering |
| 3 | [phase-3.md](phase-3.md) | Done | Dual pane, tabs, breadcrumbs, keyboard nav |
| 4 | *(not started)* | Planned | Copy, move, delete, rename, batch rename |
| 5 | *(not started)* | Planned | Fuzzy search, command palette, hotkeys |
| 6 | *(not started)* | Planned | Themes, settings, installer, release |

---

## Current codebase (after Phase 3)

- **Entry / window**: `src/main.rs` – `main()`, `AppState` (left_pane, right_pane, focused_pane, split_x), `wndproc`, WM_SIZE/WM_PAINT/WM_VSCROLL/WM_MOUSEWHEEL/WM_KEYDOWN/WM_LBUTTONDOWN/WM_MBUTTONDOWN
- **Rendering**: `src/renderer.rs` – D2D, `draw_file_list_in_rect`, `draw_tab_bar`, `draw_breadcrumb`, `draw_divider`, `draw_rect_outline` (focus), begin_draw/end_draw
- **Pane**: `src/pane.rs` – `current_path`, `file_list`, `tabs: Vec<Tab>`, `active_tab_index`, `load_path`, `switch_tab`, `close_tab`, `ensure_one_tab`
- **File list**: `src/file_list.rs` – `entries`, `scroll_offset`, `visible_range()`, `scroll_by`/`scroll_to`, `set_selection`, sort (Name/Size/Date, Asc/Desc)
- **FS**: `src/fs.rs` – `FileEntry`, `list_dir(path)`, `current_directory()`, `path_parent`, `path_join`, `path_segments`, `path_from_segments`
- **Icons**: `src/icons.rs` – `IconCache` (stub; drawing deferred)
- **Stubs**: `src/search.rs`, `src/palette.rs`, `src/config.rs`, `src/theme.rs` – placeholders for later phases

---

## Conventions

- **Tests**: After each phase, add tests; before starting the next phase, run tests and fix any failures. See [.cursor/rules/phase-tests.mdc](../.cursor/rules/phase-tests.mdc).
- **Docs**: After each phase, add or update `docs/phase-N.md` and this `docs/INDEX.md`.

---

## Running the app

- **Requirement**: Rust toolchain (e.g. [rustup](https://rustup.rs/)), Windows 10+.
- **If `cargo` is not recognized**: Add Rust to PATH (e.g. `%USERPROFILE%\.cargo\bin`) or open a terminal where Rust was installed (e.g. “Developer PowerShell for VS” after installing rustup). See [README](../README.md#cargo-not-found).
- **Build**: `cargo build --release`
- **Run**: `cargo run --release`

---

*Last updated: after Phase 3 (navigation and tabs).*
