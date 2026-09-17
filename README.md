# File Xplorer

A fast, native, dual-pane file manager for Windows. Win32 + Direct2D, no runtime,
no framework, ~275 KB.

**Full context and phase docs:** see **[docs/INDEX.md](docs/INDEX.md)**.

---

## Requirements

- **Rust** ([rustup](https://rustup.rs/))
- **Windows 10 1703+** (Direct2D, DirectWrite, WIC, `IFileOperation`, per-monitor DPI v2)

## Build and run

```bash
cargo run --release
```

The executable lands at `target/release/file-xplorer.exe`. It is self-contained:
no installer, no DLLs to ship.

### If `cargo` is not recognised

Install Rust from [rustup.rs](https://rustup.rs/), then open a new terminal so
`PATH` picks up `%USERPROFILE%\.cargo\bin`.

---

## What it does

**One pane by default, two on request.** `Ctrl+2` splits, `Ctrl+1` collapses back.
Every tab keeps its own listing, scroll position, selection, and back/forward
history, so switching tabs is instant.

**Real file operations.** Copy, move, delete, rename, and new-folder all run
through the shell's `IFileOperation`. That means you get the Recycle Bin, the
standard progress dialog, per-file conflict prompts ("replace / skip / keep
both"), the elevation prompt when a path needs admin rights, and Explorer's undo
stack — rather than a reimplementation of each.

**Nothing blocks the window.** Directory reads and file operations run on worker
threads. A slow network share or a folder with 100,000 files never freezes the UI.

**Nothing fails silently.** A directory that cannot be read says so in the pane.
An operation that fails raises a message box with the actual reason.

**Live filter.** Each pane's footer filters its listing as you type, with a
`12 of 340` count.

**Command palette.** `Ctrl+Shift+P` finds any command by initials, so nothing
is hidden behind a chord you have to remember.

**Recursive search.** `Ctrl+Shift+F` walks the tree on a worker thread and
streams matches in as it finds them. `*` is allowed (`*.rs`, `test*`).

**Batch rename.** `{n}` name, `{e}` extension, `{#}` counter, with a live
preview that flags collisions and illegal names before anything happens.

**The real Windows context menu.** Right-click shows this app's commands, then
everything installed software registered: 7-Zip, Git, Open with, Send to.

**Drag and drop**, both directions, interoperating with Explorer. Copy by
default, Shift to move; dropping on a folder row targets that folder.

**Folder sizes on demand.** Never automatic - recursive sizing of a whole drive
is how file managers earn a reputation for hanging.

**Live updates.** Directories are watched, so changes made elsewhere appear
without a refresh.

**Sidebar** with drives (label, free/total, and a capacity bar that turns amber
past 90%) and your standard folders, each with its real shell icon.

**Themes.** Dark and light, including the title bar. Both palettes are tested to
clear WCAG AA contrast for body and secondary text.

**Settings stick.** Pane count, theme, sidebar visibility, hidden files and the
divider position are written to `%APPDATA%\FileXplorer\settings.txt` on exit.
It is plain `key=value` text you can edit or delete; anything unreadable falls
back to defaults rather than refusing to start.

**Correct at any DPI.** The process is per-monitor-v2 aware and every size is
computed in physical pixels for the current monitor, so nothing is ever
bitmap-stretched. Dragging between a 100% and a 150% monitor rescales cleanly.

---

## Keyboard and mouse

Bindings follow Windows conventions, with the dual-pane extras on Ctrl+Shift.

| Action | Keys |
|---|---|
| Move / extend selection | `Up` `Down` `PageUp` `PageDown` `Home` `End`, `+Shift` to extend, `+Ctrl` to move the cursor only |
| Select all / clear | `Ctrl+A` / `Esc` |
| Multi-select with the mouse | `Ctrl+click` to toggle, `Shift+click` for a range |
| Open | `Enter` or double-click (folders navigate, files open in their default app) |
| Up / back / forward | `Backspace` or `Alt+Up` / `Alt+Left` / `Alt+Right`, or mouse buttons 4 and 5 |
| Command palette | `Ctrl+Shift+P` |
| Search in this folder and below | `Ctrl+Shift+F` |
| Batch rename | `Ctrl+Shift+R` |
| One pane / two panes | `Ctrl+1` / `Ctrl+2` |
| Switch pane | `Tab` |
| New / close / cycle tab | `Ctrl+T` / `Ctrl+W` / `Ctrl+Tab`, middle-click a tab to close |
| Refresh | `F5` or `Ctrl+R` |
| Cut / copy / paste | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` (interoperates with Explorer via `CF_HDROP`) |
| Copy / move to the other pane | `F6` or `Ctrl+Shift+C` / `Ctrl+Shift+M` |
| Delete | `Del` to the Recycle Bin, `Shift+Del` permanently |
| Rename / new folder | `F2` / `Ctrl+Shift+N` |
| Filter this pane | `Ctrl+F`, `Esc` to clear |
| Type-to-select | just start typing a name |
| Show hidden files | `Ctrl+H` |
| Toggle sidebar | `Ctrl+B` |
| Toggle dark / light | `Ctrl+Shift+D` |
| Context menu | right-click or `Shift+F10` |
| Sort | click a column header; click again to reverse |
| Resize panes | drag the divider, double-click it to even them up |

---

## Project layout

| File | Responsibility |
|---|---|
| `src/main.rs` | Win32 window, message dispatch, input handling, modal prompts |
| `src/layout.rs` | **All geometry.** Produces every rectangle; drawing and hit-testing both consume it |
| `src/renderer.rs` | Direct2D / DirectWrite painting, icon bitmap cache |
| `src/theme.rs` | Dark and light colour tokens |
| `src/pane.rs` | Tabs, navigation history, async load requests |
| `src/file_list.rs` | Virtual list, selection model, sorting, filtering |
| `src/fs.rs` | Directory enumeration, path helpers, name validation, drives |
| `src/ops.rs` | `IFileOperation` wrappers and clipboard interop |
| `src/icons.rs` | Shell icon cache, keyed by extension or path |
| `src/config.rs` | Settings file load/save |
| `src/palette.rs` | Command palette and its fuzzy matcher |
| `src/search.rs` | Recursive name search |
| `src/batch_rename.rs` | Rename patterns, preview, dialog |
| `src/watch.rs` | ReadDirectoryChangesW watcher |
| `src/dnd.rs` | Drag and drop (IDropTarget / IDropSource) |
| `src/shellmenu.rs` | The real shell context menu |
| `src/pidl.rs` | Shell item id lists, freed on drop |
| `src/dialog.rs` | Shared dialog font handling |

### Why `layout.rs` exists

Drawing and hit-testing used to compute geometry independently, and had already
drifted: breadcrumbs were painted at a fixed stride per crumb while clicks were
resolved by dividing the pane width evenly, so clicking a crumb navigated to the
wrong folder. Now one pass produces the rectangles and both consumers use them,
which makes that class of bug impossible. It holds no Win32 types, so all of it
is unit-tested without a window.

---

## Tests

```bash
cargo test
```

151 tests, no warnings. They cover path handling and name validation, the virtual
list and selection model, sorting and filtering, tab and history behaviour,
stale-load rejection, layout geometry and hit-testing, and palette contrast.

Layout tests assert the property that matters: **anything drawn at a rectangle
hit-tests back to itself.**

---

## Size

```powershell
(Get-Item target\release\file-xplorer.exe).Length / 1MB
```

Currently ~0.27 MB against a 3 MB budget.
