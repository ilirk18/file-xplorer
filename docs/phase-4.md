# Phase 4: File operations, multi-select, async I/O

**Status:** Done
**Deliverable:** Copy, move, delete, rename and new-folder through the Windows
shell; a real multi-selection model; directory reads and operations off the UI
thread; correct DPI handling.

This phase also fixed the defects found in the review of phases 1–3. Those are
listed at the end because several were data-loss bugs and it is worth knowing
they existed.

---

## What was built

### 1. Shell file operations (`src/ops.rs`)

Everything destructive goes through `IFileOperation` rather than `std::fs`:

- `Op::{Copy, Move, Delete, Rename, NewFolder}` describe the request.
- `perform()` builds one `IFileOperation`, sets flags, and runs it.
- `spawn()` runs that on a worker thread inside an STA (`CoInitializeEx` with
  `COINIT_APARTMENTTHREADED` — the progress UI is a window) and hands the result
  back through a callback.

Flags: `FOF_ALLOWUNDO` so operations land in Explorer's undo stack,
`FOFX_RECYCLEONDELETE` for the Recycle Bin, `FOF_WANTNUKEWARNING` on permanent
deletes, `FOFX_SHOWELEVATIONPROMPT` for protected paths, `FOF_NOCONFIRMMKDIR`
because creating an intermediate folder is implied by the copy.

Using the shell rather than reimplementing gives us, for free: the standard
progress dialog with time remaining, per-file conflict resolution, elevation,
long-path handling, and correct treatment of junctions and symlinks.

`Op::affected_dirs()` reports which directories a completed operation touched, so
only the panes actually showing one of them refresh.

### 2. Clipboard interop

`clipboard_write` publishes `CF_HDROP` (a `DROPFILES` header followed by a
double-NUL-terminated wide path list) plus the registered `Preferred DropEffect`
format carrying `DROPEFFECT_COPY` or `DROPEFFECT_MOVE`. `clipboard_read` reads
both back. Cut and copy therefore interoperate with Explorer in both directions.

### 3. Multi-selection (`src/file_list.rs`)

`selected: HashSet<u32>` with a separate `cursor` (the row the keyboard acts
from) and `anchor` (where a Shift-range measures from).

- `SelectMode::{Replace, Toggle, Range}` maps to plain / Ctrl / Shift.
- The anchor deliberately stays put across successive Shift-clicks so a range
  grows and shrinks rather than restarting.
- `ensure_visible` scrolls the minimum distance to bring a row on screen.
- Selection and cursor are preserved by name across a refresh and across a
  re-sort.

### 4. Async directory reads (`src/pane.rs`)

A pane never reads a directory itself. It issues a `LoadRequest` carrying a
`tab_id` and a monotonically increasing `generation`; `main.rs` runs
`fs::list_dir` on a worker thread and posts the result back as
`WM_APP_DIR_LOADED`. `finish_load` drops results whose generation is stale or
whose tab has closed.

Without the generation guard, a slow network folder would land after the user had
already navigated elsewhere and overwrite the current view.

### 5. Name validation (`src/fs.rs`)

`validate_file_name` rejects path separators, drive colons, wildcards, control
characters, `.` and `..`, reserved device names (`CON`, `NUL`, `COM1`–`COM9`,
`LPT1`–`LPT9`, whatever the extension), trailing dots and spaces, and names over
255 characters. `NameError::message()` explains each in a sentence.

### 6. DPI correctness (`build.rs`, `app.manifest`, `layout::Metrics`)

The manifest declares `PerMonitorV2`, long-path awareness, UTF-8, and Common
Controls v6, and `build.rs` embeds it with `/MANIFEST:EMBED`. Declaring it in the
manifest rather than at runtime means the process is DPI-aware before the first
window exists.

Every size comes from `Metrics::for_dpi`, and the Direct2D render target runs at
96 DPI so one DIP is one physical pixel. `WM_DPICHANGED` rescales the metrics,
recreates the text formats, and honours the suggested window rect.

### 7. No console window

`#![windows_subsystem = "windows"]`. Previously a terminal opened alongside the
app, which is the first thing anyone noticed.

### 8. Dark title bar

`DWMWA_USE_IMMERSIVE_DARK_MODE`, applied on creation and on every theme change.

---

## Defects fixed from phases 1–3

**Data loss**

- `copy_path` used `fs::copy`, which silently truncates an existing destination.
  Copying onto an existing name destroyed it with no prompt.
- Delete was `remove_dir_all`: permanent, never the Recycle Bin.
- `copy_path`/`delete_path` followed directory junctions, because `is_dir()`
  resolves reparse points. A junction loop meant unbounded recursion.
- `rename_path` did not validate the name, so `..\foo` relocated the file.
- Every filesystem call was `let _ = `, so permission denied, file in use and
  disk full all silently did nothing.

**Correctness**

- Breadcrumb clicks navigated to the wrong folder: hit-testing divided the pane
  width evenly among segments while the renderer used a fixed 70px stride.
  Fixed structurally in phase 5 by `layout.rs`.
- `path_segments` emitted a bare `\` crumb between the drive and the first
  folder, because `Path::components()` yields `Prefix` and `RootDir` separately.
- Arrow-Up re-centred the viewport on every press; Down did not. Now both scroll
  minimally.
- Arrow-Down in a fresh folder skipped the first entry (`unwrap_or(0) + 1`).
- Files could not be opened at all — there was no `ShellExecuteW`.
- No way to reach another volume: `path_parent("C:\")` is `None` and there was
  no drive UI.
- Tabs could never be created: `tabs.push` existed only inside `ensure_one_tab`,
  which only ran when the list was empty, which never happened. Closing the last
  tab was a silent no-op, and its test passed vacuously.
- The divider was not draggable despite all the supporting constants existing.
- There was no refresh binding; F5 was bound to copy.
- `WM_MOUSELEAVE` is not exported by the `windows` crate, so the match arm bound
  it as a variable and silently shadowed every message arm after it.

**Performance**

- All I/O was on the UI thread.
- `WM_MOUSEMOVE` invalidated the whole window on every pixel of movement.
- Every `WM_SIZE` destroyed and rebuilt the render target and all eleven brushes.

**Hygiene**

- `SetScrollInfo`, `InvalidateRect`, `ScreenToClient` were hand-redeclared via
  `extern "system"` with a hand-rolled `POINT`, despite the crate providing them.
- The rename dialog kept its result in a `static mut`, never disabled its parent
  (so it was not modal and could be reopened recursively), and ran a nested
  message loop that swallowed `WM_QUIT` — the app could not be closed while it
  was open.
- `use_stubs()` instantiated every empty module at startup purely to silence
  dead-code warnings. Removed along with the empty stubs.
- `search_path`'s long-path branch was dead: both arms produced the same string.
- `IconCache` leaked one `HICON` per extension; it now destroys them on drop.

---

## Tests added

`fs`: path round-tripping including UNC, root-collapsing segments, name
validation (traversal, reserved names, trailing dots), size formatting,
long-path prefixing, drive capacity maths.

`file_list`: folders-first and natural-number sorting, first Down selects row 0,
minimal-scroll `ensure_visible`, range/toggle selection, anchor stability,
selection preserved across refresh and re-sort.

`pane`: tabs reachable and independent, closing the last tab resets it, stale
loads discarded, history back/forward and truncation, navigate-up selects the
folder just left, load errors reported.

`ops`: `affected_dirs` for each operation, `DropEffect` values matching the shell.

---

*Last updated: after Phase 4.*
