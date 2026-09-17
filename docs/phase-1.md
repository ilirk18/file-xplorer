# Phase 1: Scaffold & Window

**Status:** Done  
**Deliverable:** Bare Win32 window, Direct2D context, build pipeline. No file listing yet.

---

## What was built

1. **Cargo project**
   - `Cargo.toml`: `windows` crate (Win32, Direct2D, Dxgi); release profile with `opt-level = "z"`, LTO, strip.
   - Single binary crate; no workspace.

2. **Win32 window** (`src/main.rs`)
   - `RegisterClassExW` with `CS_HREDRAW | CS_VREDRAW`, `CreateWindowExW` with `WS_OVERLAPPEDWINDOW`.
   - Message loop: `GetMessageW` / `TranslateMessage` / `DispatchMessageW`.
   - **WM_NCCREATE**: store app pointer in `GWLP_USERDATA`.
   - **WM_DESTROY**: `PostQuitMessage(0)`.
   - **WM_SIZE**: store client width/height, call `renderer.resize(hwnd, width, height)`, `InvalidateRect`.
   - **WM_PAINT**: call `renderer.draw_frame(...)`; on error, resize and redraw once.

3. **Direct2D renderer** (`src/renderer.rs`)
   - `D2D1CreateFactory(SINGLE_THREADED)`.
   - `CreateHwndRenderTarget` on resize; clear to solid dark gray in `draw_frame`.
   - No text or file list yet (Phase 2 extends this).

4. **Module stubs**
   - `pane`, `file_list`, `fs`, `icons`, `search`, `palette`, `config`, `theme` – minimal types so the crate compiles and structure is clear.

---

## Key files (Phase 1)

| File | Role |
|------|------|
| `src/main.rs` | Entry, window class, CreateWindowExW, message loop, wndproc (WM_SIZE, WM_PAINT) |
| `src/renderer.rs` | D2D factory, Hwnd render target, clear background |
| `Cargo.toml` | windows features, release profile |

---

## Tests (Phase 1)

- No automated tests added in Phase 1 (optional: add a trivial test that the binary builds, or a small unit test for a pure function if introduced later).
- Before starting Phase 2, any Phase 1 tests would be run; none were required for this phase.

---

## Next

Phase 2 adds: directory enumeration, `FileEntry`, virtual list, scrollbar, and drawing the file list in the same window. See [phase-2.md](phase-2.md) and [INDEX.md](INDEX.md).
