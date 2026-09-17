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
| 6 | [phase-6.md](phase-6.md) | Done | Palette, search, drag & drop, shell menu, watching, batch rename, settings |
| 8 | [phase-8.md](phase-8.md) | Done | Themed context menu and tab strip, Ctrl+L, terminal, pins, autosave, tab moving, tree reveal, column resize, content compare, archiving |
| 9+ | [phase-9-plan.md](phase-9-plan.md) | Planned | Nested layouts, view modes, default manager, and the rest of the File Pilot gap |
| 7 | [phase-7.md](phase-7.md) | Done | Split `main.rs`, four panes, undo, content search, session restore, Type column, folder tree, archives |

**Ideas (not phased yet):** [file-pilot-comparison.md](file-pilot-comparison.md) · [powertoys-inspired-ideas.md](powertoys-inspired-ideas.md)

---

## Current codebase

| Module | Responsibility |
|---|---|
| `src/main.rs` | Window creation, `wndproc`, message dispatch, painting |
| `src/app.rs` | `AppState`, worker-thread spawners, shared helpers |
| `src/input.rs` | Mouse and keyboard handlers |
| `src/commands.rs` | The one `COMMANDS` table, context menu, file operations, undo |
| `src/prompt.rs` | Modal single-line text prompt, with shell path completion |
| `src/rename.rs` | The inline rename editor: a real EDIT control over the name cell |
| `src/preview.rs` | Shell thumbnails: the inspector's preview, the icon view's cells, and the bounded cache behind them |
| `src/menu.rs` | The context menu's own owner-draw painting |
| `src/layout.rs` | Every rectangle in the UI, plus `hit_test`. No Win32 types, fully unit-tested |
| `src/renderer.rs` | Direct2D/DirectWrite drawing, WIC icon bitmaps, `TextMeasurer` |
| `src/theme.rs` | `Palette` tokens for dark and light, with contrast tests |
| `src/pane.rs` | `Tab` (listing, history, scroll), `Pane`, `LoadRequest`/`finish_load` generation guarding |
| `src/file_list.rs` | Virtual list, multi-select, sort (folders first, natural order), live filter |
| `src/fs.rs` | `list_dir`, path helpers, `validate_file_name`, `drives()`, size/date formatting |
| `src/ops.rs` | `IFileOperation` for copy/move/delete/rename/new-folder, `CF_HDROP` clipboard, `ShellExecuteW` |
| `src/icons.rs` | `HICON` cache keyed by extension or path, handles destroyed on drop |
| `src/config.rs` | `key=value` settings file: pane count, theme, sidebar, hidden files, splits, window box, open tabs |
| `src/search.rs` | Recursive name and content search |
| `src/tree.rs` | The sidebar's folder tree: flat rows from a set of expanded paths |
| `src/archive.rs` | Archives browsed as folders, via the 7-Zip CLI |
| `src/palette.rs` | Command palette and its fuzzy matcher |
| `src/keys.rs` | Chords, and which command each runs. No Win32 types, fully unit-tested |
| `src/batch_rename.rs` | Rename patterns, preview, dialog |
| `src/watch.rs` | `ReadDirectoryChangesW` watcher, one per pane |
| `src/dnd.rs` | Drag and drop (`IDropTarget` / `IDropSource`) |
| `src/shellmenu.rs` | The real shell context menu |
| `src/shellns.rs` | This PC, the Recycle Bin and Network: browsing the shell namespace |
| `src/uia.rs` | UI Automation providers, so a screen reader can read the listing |
| `src/pidl.rs` | Shell item id lists, freed on drop |
| `src/dialog.rs` | Shared dialog font handling |
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
6. **A pane is an index, not a position.** `PaneId(usize)` addresses one of up
   to `MAX_PANES` entries in the pane array. Where it sits on screen is what
   the layout tree says, and nothing may infer one from the other.
7. **Archives and the shell namespace are read-only**, and one guard in
   `spawn_op_tagged` enforces it for every destructive path, including ones
   nobody thought about.

---

## Conventions

- **Tests**: after each phase add tests; before the next phase run them and fix
  failures. See [.cursor/rules/phase-tests.mdc](../.cursor/rules/phase-tests.mdc).
- **Docs**: after each phase add or update `docs/phase-N.md` and this index.

Current: **267 tests, zero warnings** (`cargo test`).

---

## Running

- **Build**: `cargo build --release`
- **Run**: `cargo run --release`
- Requires Windows 10 1703+ and a Rust toolchain.

---

*Last updated: after Phase 8.*
