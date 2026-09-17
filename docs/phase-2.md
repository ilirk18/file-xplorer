# Phase 2: File Listing

**Status:** Done  
**Deliverable:** Scrollable virtual file list with directory enumeration, sort (name/size/date), scrollbar, and Direct2D text rendering.

---

## What was built

1. **Directory enumeration** (`src/fs.rs`)
   - **FileEntry**: `name`, `size`, `modified` (u64 FILETIME), `is_dir`, `extension`.
   - **list_dir(path)**: `FindFirstFileW` / `FindNextFileW` with long-path prefix `\\?\` when needed; skips `.` and `..`.
   - **current_directory()**: `GetCurrentDirectoryW` for initial path.

2. **Icon cache** (`src/icons.rs`)
   - **IconCache**: `get_icon(extension, is_dir)` using `SHGetFileInfo` (SHGFI_ICON | SHGFI_SMALLICON); cache keyed by extension and `<dir>`.
   - Phase 2 does not draw icons (placeholder only); icon drawing can be added later.

3. **Virtual list** (`src/file_list.rs`)
   - **FileList**: `entries`, `scroll_offset`, `row_height` (24), `selected_index`, `sort_key` (Name/Size/Date), `sort_order` (Asc/Desc).
   - **visible_range(client_height)** → (start, end).
   - **scroll_to** / **scroll_by** with clamping.
   - **set_entries(entries)**: replaces list, sorts, resets scroll/selection.
   - **sort()**: in-place sort by current key/order.

4. **Pane** (`src/pane.rs`)
   - **Pane**: `current_path`, `file_list: FileList`, `tabs` (placeholder).
   - **load_path(path)**: calls `fs::list_dir(path)`, sets `file_list.set_entries(...)`.

5. **Rendering** (`src/renderer.rs`)
   - DirectWrite: `DWriteCreateFactory`, `CreateTextFormat` (“Segoe UI”, 14pt).
   - On resize: create **text_brush** and **selection_brush** (solid colors).
   - **draw_frame(entries, start_index, selected_index, row_height)**: clear background; for each visible row draw selection rect (if selected) then file name with `DrawText`.

6. **Main** (`src/main.rs`)
   - **AppState**: `renderer`, `client_width`/`client_height`, **pane: Pane**, **initial_load_done**.
   - **ensure_initial_load()**: once, set path from `fs::current_directory()` or `C:\`, call `pane.load_path(path)`.
   - **update_scrollbar(hwnd)**: `SetScrollInfo(SB_VERT)` with nMin/nMax/nPage/nPos from file_list.
   - Window style: **WS_VSCROLL**.
   - **WM_SIZE**: ensure_initial_load, resize renderer, update_scrollbar, invalidate.
   - **WM_PAINT**: visible slice from `file_list.visible_range`, call `renderer.draw_frame(entries, start, selected_index, row_height)`.
   - **WM_VSCROLL**: SB_LINEUP/DOWN/PAGEUP/DOWN/THUMBPOSITION → scroll_by/scroll_to, update_scrollbar, invalidate.
   - **WM_MOUSEWHEEL**: scroll_by(±3), update_scrollbar, invalidate.

---

## Key files (Phase 2)

| File | Role |
|------|------|
| `src/fs.rs` | FileEntry, list_dir (long-path), current_directory |
| `src/icons.rs` | IconCache, SHGetFileInfo (cache only in Phase 2) |
| `src/file_list.rs` | FileList, visible_range, scroll_to/scroll_by, sort |
| `src/pane.rs` | Pane, load_path |
| `src/renderer.rs` | DirectWrite, text/selection brushes, draw_frame(entries, …) |
| `src/main.rs` | AppState + pane, WS_VSCROLL, WM_VSCROLL/WM_MOUSEWHEEL, scrollbar, paint file list |

---

## Cargo.toml changes (Phase 2)

- Added windows features: `Win32_Graphics_DirectWrite`, `Win32_Storage_FileSystem`, `Win32_UI_Shell`, `Win32_System_Time`.

---

## Tests (Phase 2)

- Per project rule: after Phase 2, tests should be added (e.g. unit tests for `list_dir` on a temp dir, `FileList::visible_range` / scroll clamping, sort order).
- Before starting Phase 3: run `cargo test`; fix any failures before continuing.

---

## Next

Phase 3: dual pane, tabs, breadcrumb bar, keyboard navigation (arrows, Enter, Backspace, Alt+Left, Tab). See [INDEX.md](INDEX.md).
