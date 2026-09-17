// Win32 window, message dispatch, and the glue between input and state.
//
// Design notes worth knowing before editing:
//
//  * No console window: the subsystem attribute below is what stopped a
//    terminal opening alongside the app.
//  * Geometry is never computed here. `layout::Layout` produces every
//    rectangle, this file hit-tests against those same rectangles, and the
//    renderer paints them. Adding a new clickable thing means adding it to the
//    layout, not writing new arithmetic in both places.
//  * Directory reads and file operations run on worker threads and come back
//    as posted messages, so the window never blocks on a slow volume.
//  * Nothing is silently swallowed: a failed listing shows in the pane, a
//    failed operation shows in a message box.

#![windows_subsystem = "windows"]

mod app;
mod archive;
mod batch_rename;
mod commands;
mod config;
mod dialog;
mod dnd;
mod file_list;
mod fs;
mod icons;
mod input;
mod keys;
mod layout;
mod menu;
mod ops;
mod palette;
mod pane;
mod pidl;
mod preview;
mod prompt;
mod rename;
mod renderer;
mod search;
mod shellmenu;
mod theme;
mod tree;
mod watch;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use std::collections::HashSet;

use app::*;
use commands::show_context_menu;
use config::{Config, UNSET};
use input::*;
use file_list::{SelectMode, SortOrder};
use layout::{Hit, Metrics, PaneId, Rect, MAX_PANES};
use pane::Pane;
use renderer::{PaneView, SidebarView};


// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    unsafe {
        // OleInitialize rather than CoInitializeEx: it enters the same
        // single-threaded apartment the shell interfaces need, and additionally
        // sets up the drag-and-drop machinery RegisterDragDrop requires.
        let _ = OleInitialize(None);

        let instance = GetModuleHandleW(None)?;
        let class_name = wide("FileXplorerWindow");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // Null background: we paint every pixel with Direct2D, and letting
            // the system erase first would only flicker.
            hbrBackground: HBRUSH::default(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        // Restore the saved box, but only if it still lands on a monitor:
        // a window restored onto a display that has since been unplugged is
        // unreachable.
        let cfg = Config::load();
        let (x, y, w, h) = if cfg.win_x == UNSET {
            (CW_USEDEFAULT, CW_USEDEFAULT, 1200, 760)
        } else {
            let r = RECT {
                left: cfg.win_x,
                top: cfg.win_y,
                right: cfg.win_x + cfg.win_w,
                bottom: cfg.win_y + cfg.win_h,
            };
            if MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_invalid() {
                (CW_USEDEFAULT, CW_USEDEFAULT, cfg.win_w, cfg.win_h)
            } else {
                (cfg.win_x, cfg.win_y, cfg.win_w, cfg.win_h)
            }
        };

        let title = wide("File Xplorer");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let _ = ShowWindow(
            hwnd,
            if cfg.maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOW
            },
        );
        let _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            // The inline rename box is a real EDIT and swallows Enter and
            // Escape before the window procedure can see them.
            if let Some(state) = state_of(hwnd) {
                if rename::handle_key(hwnd, state, &msg) {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        OleUninitialize();
    }
    Ok(())
}

/// The inspector's facts, built before `paint` destructures `state` and
/// therefore owning its strings.
struct InspectorView {
    name: Option<String>,
    kind: String,
    size: String,
    modified: String,
}

/// Rows a full wheel notch moves. Windows' own default; reading
/// SPI_GETWHEELSCROLLLINES would honour the user's setting, which nothing has
/// asked for yet.
const WHEEL_ROWS: i32 = 3;

/// The `AppState` hanging off the main window.
///
/// The message pump needs it before dispatch, where the window procedure's own
/// borrow does not exist yet. Read-only: anything that changes state goes
/// through a posted message like every other input.
unsafe fn state_of(hwnd: HWND) -> Option<&'static AppState> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const AppState).as_ref()
}

/// Match the title bar to the app's theme. Without this a dark app sits under a


// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let dpi = GetDpiForWindow(hwnd).max(96);
            match AppState::new(dpi) {
                Ok(state) => {
                    let boxed = Box::into_raw(Box::new(state));
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, boxed as isize);
                }
                Err(_) => return LRESULT(0), // Abort creation.
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        // Our own owner-drawn entries get first refusal; everything else is
        // the shell's, whose handlers build submenus and owner-draw their
        // items only if these reach them while the menu is on screen.
        if let Some(result) = menu::handle_menu_msg(msg, wparam, lparam) {
            return result;
        }
        if let Some(result) = shellmenu::handle_menu_msg(msg, wparam, lparam) {
            return result;
        }

        let state = &mut *ptr;
        handle(state, hwnd, msg, wparam, lparam)
            .unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
    }
}

/// Returns None for messages we do not handle, so the caller falls through to
/// DefWindowProc. Keeping that decision in one place avoids the easy mistake of
/// swallowing a message by returning LRESULT(0) from an unhandled arm.
fn handle(
    state: &mut AppState,
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match msg {
        WM_CREATE => {
            apply_titlebar_theme(hwnd, state.theme);
            state.drop_target = dnd::Registration::new(hwnd, WM_APP_DROPPED);
            state.apply_col_widths();
            unsafe { SetTimer(Some(hwnd), TIMER_AUTOSAVE, AUTOSAVE_MS, None) };
            let start = fs::current_directory().unwrap_or_else(|_| FALLBACK_PATH.to_string());
            // Every pane opens something, including the ones that are off
            // screen: pressing Ctrl+2 should reveal a folder, not a blank panel.
            let session = std::mem::take(&mut state.session);
            for pid in (0..MAX_PANES).map(PaneId) {
                let saved = &session.tabs[pid.0];
                let req = match state
                    .pane_mut(pid)
                    .restore(saved, session.active_tab[pid.0])
                {
                    Some(req) => req,
                    None => state.pane_mut(pid).navigate(&start),
                };
                let (key, asc) = session.sort[pid.0];
                let list = state.pane_mut(pid).list_mut();
                list.sort_key = sort_key_from_id(key);
                list.sort_order = if asc { SortOrder::Asc } else { SortOrder::Desc };
                spawn_dir_load(hwnd, pid, req);
            }
            Some(LRESULT(0))
        }

        WM_DESTROY => {
            // Drop the watchers first: their threads post to this window.
            state.watchers = (0..MAX_PANES).map(|_| None).collect();
            state.drop_target = None;
            state.config(hwnd).save();
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }

        // We paint every pixel; erasing first would only flicker.
        WM_ERASEBKGND => Some(LRESULT(1)),

        WM_SIZE => {
            // The editor is placed at a rectangle a resize invalidates, and
            // there is nothing sensible to do with a box that no longer sits
            // over its row. Committing here would apply a half-typed name to
            // a window drag, so this one abandons.
            commands::finish_rename(state, hwnd, false);
            let w = loword(lparam.0 as u32).max(0);
            let h = hiword(lparam.0 as u32).max(0);
            state.client = Rect::new(0, 0, w, h);
            // Splits are fractions of the body, so a resize keeps the panes in
            // proportion; the clamp only steps in when one would get too narrow.
            state.clamp_split();
            let _ = state.renderer.resize(hwnd, w.max(0) as u32, h.max(0) as u32);
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_DPICHANGED => {
            // lParam carries the suggested new window rect for the new monitor.
            let dpi = loword(wparam.0 as u32).max(96) as u32;
            state.dpi = dpi;
            state.metrics = Metrics::for_dpi(dpi);
            let _ = state.renderer.set_dpi(dpi);
            let row_h = state.metrics.row_h as u32;
            for p in &mut state.panes {
                p.set_row_height(row_h);
            }
            state.apply_col_widths();
            let r = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            state.clamp_split();
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };
            paint(state, hwnd);
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            Some(LRESULT(0))
        }

        WM_SETCURSOR => {
            // Only override inside the client area, and only over the divider.
            if (lparam.0 as u32 & 0xFFFF) == HTCLIENT as u32 {
                let over_divider = matches!(state.hover, Some(Hit::Divider(_)))
                    || matches!(state.drag, Drag::Divider { .. });
                if over_divider {
                    unsafe {
                        if let Ok(c) = LoadCursorW(None, IDC_SIZEWE) {
                            SetCursor(Some(c));
                        }
                    }
                    return Some(LRESULT(1));
                }
            }
            None
        }

        WM_MOUSEMOVE => {
            let (x, y) = mouse_pos(lparam);
            if !state.mouse_tracking {
                let mut t = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut t);
                }
                state.mouse_tracking = true;
            }
            on_mouse_move(state, hwnd, x, y);
            Some(LRESULT(0))
        }

        WM_MOUSELEAVE => {
            state.mouse_tracking = false;
            if state.hover.is_some() {
                state.hover = None;
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_LBUTTONDOWN => {
            let (x, y) = mouse_pos(lparam);
            on_left_down(state, hwnd, x, y);
            Some(LRESULT(0))
        }

        WM_LBUTTONUP => {
            // A tab dropped on another pane moves there, keeping its history.
            if let Some((from, index, ox, oy)) = state.tab_drag.take() {
                let (x, y) = mouse_pos(lparam);
                let far = (x - ox).abs() >= DRAG_THRESHOLD || (y - oy).abs() >= DRAG_THRESHOLD;
                if let (true, Some(to)) = (far, state.layout().pane_at(x, y)) {
                    if to != from {
                        move_tab(state, hwnd, from, index, to);
                    } else if let Some(target) = tab_drop_index(state, to, x, y) {
                        if state.pane_mut(to).reorder_tab(index, target) {
                            invalidate(hwnd);
                        }
                    }
                }
            }
            // A click that never became a drag still needs to reduce a
            // multi-selection to the row that was pressed.
            if let Some((ox, oy)) = state.drag_origin.take() {
                if select_mode() == SelectMode::Replace {
                    let layout = state.layout();
                    if let Hit::Row(pid, row) = layout.hit_test(ox, oy) {
                        if state.pane(pid).list().selection_count() > 1 {
                            state.pane_mut(pid).list_mut().select(row, SelectMode::Replace);
                            invalidate(hwnd);
                        }
                    }
                }
            }
            if state.drag != Drag::None {
                state.drag = Drag::None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_LBUTTONDBLCLK => {
            let (x, y) = mouse_pos(lparam);
            on_double_click(state, hwnd, x, y);
            Some(LRESULT(0))
        }

        WM_MBUTTONDOWN => {
            let (x, y) = mouse_pos(lparam);
            if let Hit::Tab(pid, i) | Hit::TabClose(pid, i) = state.layout().hit_test(x, y) {
                state.focused = pid;
                let req = state.pane_mut(pid).close_tab(i, FALLBACK_PATH);
                start_load(hwnd, pid, req);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_XBUTTONDOWN => {
            // XBUTTON1 = back, XBUTTON2 = forward, as everywhere else.
            let button = hiword(wparam.0 as u32);
            let pid = state.focused;
            let req = if button == XBUTTON1 as i32 {
                state.pane_mut(pid).go_back()
            } else {
                state.pane_mut(pid).go_forward()
            };
            start_load(hwnd, pid, req);
            invalidate(hwnd);
            Some(LRESULT(1))
        }

        WM_MOUSEWHEEL => {
            // Same reasoning as WM_SIZE: scrolling moves the row out from
            // under the box.
            commands::finish_rename(state, hwnd, false);
            let delta = hiword(wparam.0 as u32);
            let mut pt = POINT {
                x: loword(lparam.0 as u32),
                y: hiword(lparam.0 as u32),
            };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            let layout = state.layout();
            // Over the sidebar, the wheel scrolls the sidebar. A folder tree
            // outgrows the window in two clicks, so it has to.
            if layout.sidebar.contains(pt.x, pt.y) {
                let step = state.metrics.sidebar_row_h * 3;
                state.sidebar_scroll += if delta > 0 { -step } else { step };
                state.clamp_sidebar_scroll();
                invalidate(hwnd);
                return Some(LRESULT(0));
            }
            // Scroll whatever the pointer is over, which is what people expect,
            // without stealing keyboard focus from another pane.
            let pid = layout.pane_at(pt.x, pt.y).unwrap_or(state.focused);
            // In pixels rather than rows, so a precision touchpad reporting
            // less than a notch moves the list by less than a row. A mouse
            // wheel still sends exactly WHEEL_DELTA and still moves three.
            let h = layout.pane(pid).list.h.max(0) as u32;
            let step = layout.metrics.row_h * WHEEL_ROWS;
            let dy = -delta * step / WHEEL_DELTA as i32;
            state.pane_mut(pid).list_mut().scroll_by_px(dy, h);
            state.mirror_scroll(pid);
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_CONTEXTMENU => {
            let x = loword(lparam.0 as u32);
            let y = hiword(lparam.0 as u32);
            show_context_menu(state, hwnd, x, y);
            Some(LRESULT(0))
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if on_key_down(state, hwnd, VIRTUAL_KEY(wparam.0 as u16)) {
                Some(LRESULT(0))
            } else {
                None
            }
        }

        WM_CHAR => {
            let c = char::from_u32(wparam.0 as u32).unwrap_or('\0');
            if !c.is_control() && !ctrl_down() && !alt_down() {
                on_type_ahead(state, hwnd, c);
                return Some(LRESULT(0));
            }
            None
        }

        // Enter or Escape in the inline rename box, turned into a message by
        // the pump because the EDIT control itself consumes both keys.
        rename::WM_APP_RENAME_DONE => {
            commands::finish_rename(state, hwnd, wparam.0 != 0);
            Some(LRESULT(0))
        }

        // Clicking away, or anything else that takes focus off the box, commits
        // what was typed. That is what every in-place rename in Windows does,
        // and the alternative — silently discarding it — loses work.
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if id == rename::IDC_INLINE_EDIT && code == EN_KILLFOCUS {
                commands::finish_rename(state, hwnd, true);
                return Some(LRESULT(0));
            }
            None
        }

        WM_APP_THUMB_READY => {
            let payload = unsafe {
                Box::from_raw(lparam.0 as *mut (String, Option<windows::Win32::Graphics::Gdi::HBITMAP>))
            };
            let (key, bmp) = *payload;
            state.finish_thumb(key, bmp);
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_APP_PREVIEW_READY => {
            let payload = unsafe { Box::from_raw(lparam.0 as *mut (String, preview::Preview)) };
            let (key, built) = *payload;
            // A selection that moved on while the worker read the file: drop
            // the result rather than showing the wrong file's contents.
            if state.preview_pending.as_deref() == Some(key.as_str()) {
                state.preview_pending = None;
                state.preview = Some((key, built));
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_DIR_LOADED => {
            // A listing that changed underneath the editor leaves it pointing
            // at whatever row now has that index. Abandon rather than rename
            // the wrong file.
            commands::finish_rename(state, hwnd, false);
            let payload = unsafe { Box::from_raw(lparam.0 as *mut DirLoaded) };
            let pid = payload.pid;
            let path = payload.req.path.clone();
            let h = state.list_height(pid);
            let changed = state.pane_mut(pid).finish_load(
                payload.req.tab_id,
                payload.req.generation,
                &payload.req.path,
                payload.req.kind,
                payload.result,
                h,
            );
            if changed {
                // A folder that actually opened is one worth offering again,
                // and one whose remembered sort now applies. Both are keyed on
                // the load finishing rather than on the request, so a path that
                // failed to read never joins the history.
                state.remember_visit(&path);
                if let Some((key, order)) = state.sort_for(&path) {
                    state.pane_mut(pid).list_mut().set_sort(key, order);
                }
                state.rewatch(pid, hwnd);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_DIR_CHANGED => {
            // Coalesce: restart the timer so a burst of writes collapses into
            // a single reload once things settle.
            let id = TIMER_WATCH_BASE + wparam.0.min(MAX_PANES - 1);
            unsafe { SetTimer(Some(hwnd), id, WATCH_DEBOUNCE_MS, None) };
            Some(LRESULT(0))
        }

        WM_TIMER => {
            let id = wparam.0;
            if id == TIMER_AUTOSAVE {
                // Only write when something actually changed, so an idle app
                // is not touching the disk every few seconds.
                let now = state.config(hwnd);
                if now != state.last_saved {
                    now.save();
                    state.last_saved = now;
                }
                return Some(LRESULT(0));
            }
            if (TIMER_WATCH_BASE..TIMER_WATCH_BASE + MAX_PANES).contains(&id) {
                unsafe {
                    let _ = KillTimer(Some(hwnd), id);
                }
                let pid = PaneId(id - TIMER_WATCH_BASE);
                let req = state.pane_mut(pid).refresh();
                start_load(hwnd, pid, req);
                return Some(LRESULT(0));
            }
            None
        }

        WM_APP_DROPPED => {
            let dropped = unsafe { Box::from_raw(lparam.0 as *mut dnd::Dropped) };
            on_dropped(state, hwnd, *dropped);
            Some(LRESULT(0))
        }

        WM_APP_TASK_DONE => {
            let done = unsafe { Box::from_raw(lparam.0 as *mut TaskDone) };
            if let Some(err) = &done.error {
                state.status_override = None;
                report_error(hwnd, "Failed", err);
            } else {
                state.status_override =
                    (!done.message.is_empty()).then(|| done.message.clone());
            }
            for dir in &done.refresh {
                for pid in state.visible().collect::<Vec<_>>() {
                    if pane::paths_equal(state.pane(pid).current_path(), dir) {
                        let req = state.pane_mut(pid).refresh();
                        start_load(hwnd, pid, req);
                    }
                }
            }
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_APP_DIFF_DONE => {
            let done = unsafe { Box::from_raw(lparam.0 as *mut DiffDone) };
            let n = done.differing.len();
            if let Some(tab) = state.pane_mut(done.pid).tab_mut(done.tab_id) {
                tab.file_list.differing = done.differing.into_iter().collect();
            }
            state.status_override = Some(match n {
                0 => "Every shared file matches".to_string(),
                1 => "1 file differs".to_string(),
                n => format!("{} files differ", n),
            });
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        WM_APP_SEARCH_BATCH => {
            let msg = unsafe { Box::from_raw(lparam.0 as *mut SearchBatch) };
            let applied = state.pane_mut(msg.pid).finish_search_batch(
                msg.tab_id,
                msg.batch.generation,
                msg.batch.entries,
                msg.batch.done,
            );
            if applied {
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_SIZES_DONE => {
            let done = unsafe { Box::from_raw(lparam.0 as *mut SizesDone) };
            if let Some(tab) = state.pane_mut(done.pid).tab_mut(done.tab_id) {
                tab.file_list.set_dir_sizes(done.sizes);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_OP_DONE => {
            let result = unsafe { Box::from_raw(lparam.0 as *mut ops::OpResult) };
            // Only a completed operation is worth taking back, and an undo
            // must not stack its own inverse or Ctrl+Z would just toggle.
            if result.error.is_none() && !result.aborted && wparam.0 != OP_WAS_UNDO {
                if let Some(inverse) = result.op.inverse() {
                    state.undo_stack.push(inverse);
                    if state.undo_stack.len() > UNDO_DEPTH {
                        state.undo_stack.remove(0);
                    }
                }
            }
            if let Some(err) = &result.error {
                report_error(hwnd, "File operation failed", err);
            } else if result.aborted {
                state.status_override = Some("Cancelled".into());
            }
            // Whatever the operation just created is what the user wants
            // selected when the listing comes back.
            let created = match &result.op {
                ops::Op::Rename { new_name, .. } => Some(new_name.clone()),
                ops::Op::NewFolder { name, .. } => Some(name.clone()),
                _ => None,
            };

            // Refresh only the panes actually showing an affected directory.
            for dir in result.op.affected_dirs() {
                for pid in (0..MAX_PANES).map(PaneId) {
                    if !pane::paths_equal(state.pane(pid).current_path(), &dir) {
                        continue;
                    }
                    if let Some(name) = &created {
                        state.pane_mut(pid).select_after_next_load(name);
                    }
                    let req = state.pane_mut(pid).refresh();
                    start_load(hwnd, pid, req);
                }
            }
            if !result.aborted && result.error.is_none() {
                state.status_override = None;
            }
            invalidate(hwnd);
            Some(LRESULT(0))
        }

        _ => None,
    }
}


/// Move a tab from one pane to another, keeping its listing and history.
/// Where a tab dropped at (x, y) lands within its own pane: onto another tab,
/// that tab's slot; onto the new-tab button or the strip past the last tab, the
/// end. Anywhere else is not a reorder.
fn tab_drop_index(state: &AppState, pid: PaneId, x: i32, y: i32) -> Option<usize> {
    match state.layout().hit_test(x, y) {
        Hit::Tab(p, i) if p == pid => Some(i),
        Hit::NewTab(p) if p == pid => Some(state.pane(pid).tabs.len() - 1),
        _ => None,
    }
}

fn move_tab(state: &mut AppState, hwnd: HWND, from: PaneId, index: usize, to: PaneId) {
    let Some(tab) = state.pane_mut(from).take_tab(index) else {
        return;
    };
    let req = state.pane_mut(to).adopt_tab(tab);
    start_load(hwnd, to, req);
    state.focused = to;
    state.rewatch(from, hwnd);
    state.rewatch(to, hwnd);
    invalidate(hwnd);
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn paint(state: &mut AppState, hwnd: HWND) {
    // The one place that sees the selection after every way of changing it —
    // arrows, clicks, a drag band, type-ahead, a reload. Spawns a worker at
    // most; the disk work is not done here.
    state.sync_columns();
    state.ensure_preview(hwnd);
    state.ensure_thumbs(hwnd);
    let cache = std::mem::take(&mut state.thumbs);
    state.renderer.prune_cells(&cache);
    state.thumbs = cache;
    if !state.renderer.has_target() {
        let _ = state
            .renderer
            .resize(hwnd, state.client.w.max(0) as u32, state.client.h.max(0) as u32);
    }
    let layout = state.layout();
    let (sidebar_entries, _) = state.sidebar_model();
    let current = state.focused_pane().current_path().to_string();
    let state_drag = match state.drag {
        Drag::Divider { index, .. } => Some(index),
        _ => None,
    };
    let renaming = state.rename.as_ref().map(|r| (r.pid, r.row));
    // Clipped to the rows here rather than while dragging, so the selection
    // still follows a mouse that has left the pane.
    let band = match state.drag {
        Drag::Band {
            pid,
            origin,
            cursor,
        } => Some((pid, Rect::between(origin, cursor).clamp_to(layout.pane(pid).list))),
        _ => None,
    };
    let hover = state.hover;
    let focused_side = state.focused;
    let filter_focus = state.filter_focus;
    let drives = state.drives.clone();
    // Pinned folders sit after the standard ones and share their indices, so
    // the sidebar entries the layout built still line up with this list.
    let places: Vec<(String, String)> = state
        .places
        .iter()
        .map(|p| (p.label.clone(), p.path.clone()))
        .chain(state.pins.iter().map(|p| (fs::path_leaf(p), p.clone())))
        .collect();
    let tree_rows = state.tree_rows.clone();
    let visible: Vec<PaneId> = state.visible().collect();
    let counts: Vec<String> = visible.iter().map(|p| state.counts_text(*p)).collect();
    // Compare mode diffs each pane against the one to its right, wrapping. With
    // two panes that is exactly "the other pane".
    let comparing = state.compare && state.pane_count > 1;
    let names: Vec<HashSet<String>> = if comparing {
        visible.iter().map(|p| state.pane(*p).list().names()).collect()
    } else {
        Vec::new()
    };
    update_title(state, hwnd);

    // Built before the destructure: it reads the focused pane's cursor entry,
    // which `panes` would otherwise be holding.
    let inspector = {
        let entry = state.focused_pane().list().cursor_entry();
        InspectorView {
            name: entry.map(|e| e.name.clone()),
            kind: entry.map(|e| e.type_display()).unwrap_or_default(),
            size: entry.map(|e| e.size_display()).unwrap_or_default(),
            modified: entry.map(|e| e.date_display()).unwrap_or_default(),
        }
    };

    // Destructure so the renderer can be borrowed mutably while the panes are
    // borrowed immutably. Borrowing through `state` would conflict.
    let AppState {
        renderer,
        panes,
        preview,
        thumbs,
        grid,
        ..
    } = state;
    let grid = *grid;

    renderer.begin();
    renderer.draw_sidebar(
        &layout,
        &SidebarView {
            entries: &sidebar_entries,
            drives: &drives,
            places: &places,
            current_path: &current,
            tree: &tree_rows,
            hover,
        },
    );

    renderer.draw_inspector(
        &layout,
        &renderer::InspectorView {
            name: inspector.name.as_deref(),
            kind: inspector.kind,
            size: inspector.size,
            modified: inspector.modified,
            preview: preview.as_ref().map(|(k, p)| (k.as_str(), p)),
        },
    );

    for &pid in &visible {
        let pane: &Pane = &panes[pid.0];
        let tabs: Vec<String> = pane.tabs.iter().map(|t| t.label()).collect();
        let crumbs = fs::path_segments(pane.current_path());
        let view = PaneView {
            pid,
            layout: layout.pane(pid),
            tab_labels: &tabs,
            active_tab: pane.active_tab_index,
            crumbs: &crumbs,
            list: pane.list(),
            focused: focused_side == pid,
            loading: pane.active().loading,
            error: pane.active().error.as_deref(),
            hover,
            can_back: pane.active().can_go_back(),
            can_forward: pane.active().can_go_forward(),
            can_up: fs::path_parent(pane.current_path()).is_some(),
            filter_focused: filter_focus == Some(pid),
            renaming: renaming.filter(|(p, _)| *p == pid).map(|(_, row)| row),
            thumbs: grid.then_some((thumbs, pane.current_path())),
            band: band.filter(|(p, _)| *p == pid).map(|(_, r)| r),
            counts: &counts[pid.0],
            other_names: comparing.then(|| &names[(pid.0 + 1) % names.len()]),
        };
        renderer.draw_pane(layout.metrics, &view);
    }

    let active_divider = match (state_drag, hover) {
        (Some(i), _) => Some((i, true)),
        (None, Some(Hit::Divider(i))) => Some((i, false)),
        _ => None,
    };
    renderer.draw_dividers(&layout, active_divider);

    if renderer.end().is_err() {
        // Device lost: drop everything and come back next frame.
        renderer.discard_target();
        invalidate(hwnd);
    }
}
