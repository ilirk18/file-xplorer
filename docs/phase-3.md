# Phase 3: Navigation and Tabs

**Status:** Done  
**Deliverable:** Dual-pane layout, tabs per pane, breadcrumb bar, keyboard navigation (arrows, Enter, Backspace, Tab), mouse selection and tab/breadcrumb interaction, scroll-by-pane under cursor.

---

## What was built

1. **Path helpers** (`src/fs.rs`)
   - **path_parent(path)** → `Option<String>`: parent directory; `None` at root (e.g. `C:\`).
   - **path_join(path, name)** → `String`: join path with name (for Enter on a directory).
   - **path_segments(path)** → `Vec<String>`: breadcrumb segments (e.g. `C:\Users\Foo` → `["C:", "Users", "Foo"]`).
   - **path_from_segments(segments, up_to_index)** → `String`: build path from segments for breadcrumb click.

2. **Tabs** (`src/pane.rs`)
   - **Tab**: `{ path: String }`.
   - **Pane**: `tabs: Vec<Tab>`, `active_tab_index: usize`; one tab on creation.
   - **switch_tab(i)**: set active tab and load its path.
   - **close_tab(i)**: remove tab; if last tab, **ensure_one_tab()** adds one (e.g. `C:\`).
   - **load_path** updates the current tab’s path.

3. **Dual-pane AppState** (`src/main.rs`)
   - **left_pane**, **right_pane**: `Pane`, **focused_pane**: `FocusedPane` (Left | Right), **split_x** (pixel).
   - **list_height()** = client height − tab bar (24) − breadcrumb (20).
   - **left_pane_rect()** / **right_pane_rect()**; **clamp_split()** on WM_SIZE (min pane width 100px, divider 4px).
   - **ensure_initial_load()**: load initial directory into both panes.

4. **Renderer** (`src/renderer.rs`)
   - **draw_file_list_in_rect(rect, entries, start, selected, row_height)**: clip to rect, draw rows.
   - **draw_divider(x, height)**: vertical line.
   - **draw_tab_bar(rect, tab_labels, active_index)** and **draw_breadcrumb(rect, segments)**.
   - **draw_rect_outline(rect, stroke_width)**: focus indicator.
   - **begin_draw** / **clear_background** / **end_draw** for multi-region paint.

5. **WM_PAINT**
   - Clear once; draw left pane (tab bar, breadcrumb, list in rect), right pane (same), divider, focus outline on focused pane’s list rect.

6. **Scroll**
   - **WM_VSCROLL** and **WM_MOUSEWHEEL**: act on **focused** pane’s file_list using **list_height**.
   - **WM_MOUSEWHEEL**: cursor position (ScreenToClient) used to scroll the pane under cursor and set focus to that pane.
   - **update_scrollbar(hwnd)** uses focused pane’s list and list_height.

7. **Keyboard** (WM_KEYDOWN)
   - **VK_UP** / **VK_DOWN**: move selection; scroll to keep selection visible.
   - **VK_RETURN**: open directory (path_join + load_path); files deferred.
   - **VK_BACK**: go up (path_parent + load_path).
   - **VK_TAB**: toggle focused_pane (Left ↔ Right).

8. **Mouse**
   - **WM_LBUTTONDOWN**: hit-test pane (split_x). In list_rect: set selection by row; in tab bar: switch_tab; in breadcrumb: path_from_segments + load_path. Click in a pane sets focused_pane.
   - **WM_MBUTTONDOWN**: on tab bar, close_tab at hit index.
   - Focus indicator: 2px outline on focused pane’s list rect.

---

## Tests

- **fs**: `test_path_parent`, `test_path_join`, `test_path_segments`, `test_path_from_segments`.
- **file_list**: `test_visible_range`, `test_scroll_by`.
- **pane**: `test_switch_tab`, `test_close_tab_ensures_one`.

Run: `cargo test`

---

## Out of scope (Phase 3)

- File operations (copy, move, delete, rename) – Phase 4.
- Fuzzy search, command palette – Phase 5.
- Opening files (e.g. ShellExecute) – Phase 3 opens directories only.

---

*Last updated: after Phase 3 (navigation and tabs).*
