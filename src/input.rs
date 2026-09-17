// Mouse and keyboard.
//
// Every handler here resolves what the user pointed at through
// `Layout::hit_test`, against the very rectangles the renderer painted. No
// handler computes geometry of its own, which is what keeps clicking and
// drawing from drifting apart.

use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::commands::*;
use crate::file_list::SelectMode;
use crate::layout::{self, Hit, NavButton, PaneId};
use crate::{dnd, fs, ops, pane};


// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

/// Files were dropped on the window: work out where, and run the operation.
pub fn on_dropped(state: &mut AppState, hwnd: HWND, dropped: dnd::Dropped) {
    let mut pt = POINT {
        x: dropped.screen.0,
        y: dropped.screen.1,
    };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
    }
    let layout = state.layout();
    let hit = layout.hit_test(pt.x, pt.y);
    let pid = match hit {
        Hit::Row(s, _) | Hit::ListBackground(s) => s,
        _ => return, // Dropped on chrome, not on a listing.
    };

    // Dropping onto a folder row targets that folder, which is what the
    // highlight under the cursor implies; anywhere else means this folder.
    let mut dest = state.pane(pid).current_path().to_string();
    if let Hit::Row(_, row) = hit {
        if let Some(entry) = state.pane(pid).list().entries.get(row as usize) {
            if entry.is_dir {
                dest = fs::child_path(&dest, entry);
            }
        }
    }
    if dest.is_empty() {
        return;
    }

    state.focused = pid;
    state.status_override = Some(if dropped.move_it {
        "Moving\u{2026}".into()
    } else {
        "Copying\u{2026}".to_string()
    });
    let op = if dropped.move_it {
        ops::Op::Move {
            sources: dropped.paths,
            dest_dir: dest,
        }
    } else {
        ops::Op::Copy {
            sources: dropped.paths,
            dest_dir: dest,
        }
    };
    spawn_op(hwnd, op);
}

pub fn on_mouse_move(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
    match state.drag {
        Drag::Divider { index, grab_offset } => {
            // The grab offset is along whichever axis the divider runs, so one
            // subtraction serves both: the other coordinate is ignored by
            // `split_fraction_at` anyway.
            let f = state
                .layout()
                .split_fraction_at(index, x - grab_offset, y - grab_offset);
            state.set_divider(index, f);
            invalidate(hwnd);
            return;
        }
        Drag::Column { key, start_x, start_w } => {
            if let Some(i) = AppState::col_index(key) {
                // The edge being dragged is the column's *left* one, so moving
                // left makes the column wider.
                // Back to stored units, which are pixels at 100% text.
                let dip = ((start_x - x) as f32 / state.col_scale()).round() as i32;
                state.col_widths[i] = (start_w + dip).clamp(40, 400);
                state.apply_col_widths();
                invalidate(hwnd);
            }
            return;
        }
        Drag::Scrollbar { pid, grab_offset } => {
            let layout = state.layout();
            let p = layout.pane(pid);
            let total = state.pane(pid).list().total_rows();
            let visible = p.visible_rows;
            let offset = layout::scroll_offset_for_thumb_y(
                p.scrollbar,
                layout.metrics,
                total,
                visible,
                y - grab_offset,
            );
            let h = p.list.h.max(0) as u32;
            state.pane_mut(pid).list_mut().scroll_to(offset, h);
            invalidate(hwnd);
            return;
        }
        Drag::Band { pid, origin, .. } => {
            state.drag = Drag::Band {
                pid,
                origin,
                cursor: (x, y),
            };
            let layout = state.layout();
            let p = layout.pane(pid);
            let list = state.pane(pid).list();
            let (scroll, total) = (list.scroll_px(), list.total_rows());
            let covered = p.band_indices(layout::Rect::between(origin, (x, y)), scroll, total);
            // The moving end is the entry under the mouse when there is one,
            // and otherwise the last one the band reached.
            let moving = p
                .cell_at(x, y, scroll, total)
                .or_else(|| covered.last().copied());
            state
                .pane_mut(pid)
                .list_mut()
                .select_indices(&covered, moving);
            invalidate(hwnd);
            return;
        }
        Drag::None => {}
    }

    // Past the threshold with the button down on a selected row: start a drag.
    if let Some((ox, oy)) = state.drag_origin {
        let moved = (x - ox).abs() >= DRAG_THRESHOLD || (y - oy).abs() >= DRAG_THRESHOLD;
        let button_down = unsafe { (GetKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 };
        if moved && button_down {
            state.drag_origin = None;
            let pid = state.focused;
            let paths = state.pane(pid).selected_paths();
            if !paths.is_empty() {
                // Blocks until the drag ends; OLE runs its own message loop.
                let moved_out = dnd::start_drag(&paths);
                if moved_out {
                    let req = state.pane_mut(pid).refresh();
                    start_load(hwnd, pid, req);
                }
            }
            return;
        }
        if !button_down {
            state.drag_origin = None;
        }
    }

    // Resolve the row under the cursor so hovering a row highlights it.
    let layout = state.layout();
    let mut hit = layout.hit_test(x, y);
    if let Hit::ListBackground(pid) = hit {
        let p = layout.pane(pid);
        let list = state.pane(pid).list();
        if let Some(row) = p.cell_at(x, y, list.scroll_px(), list.total_rows()) {
            hit = Hit::Row(pid, row);
        }
    }
    let new_hover = Some(hit);

    // Repaint only when the hover target actually changed. The old code
    // invalidated the whole window on every pixel of mouse movement.
    if state.hover != new_hover {
        state.hover = new_hover;
        invalidate(hwnd);
    }
}

pub fn on_left_down(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
    // Any new action retires the previous one-shot message ("3 items copied"),
    // which otherwise sat in the footer until some later operation finished.
    state.status_override = None;
    let layout = state.layout();
    let hit = layout.hit_test(x, y);

    // Clicking anything other than the filter box returns keystrokes to the list.
    if !matches!(hit, Hit::Filter(_)) {
        state.filter_focus = None;
    }

    match hit {
        Hit::Divider(i) => {
            let d = layout.dividers[i];
            state.drag = Drag::Divider {
                index: i,
                grab_offset: if d.vertical { x - d.rect.x } else { y - d.rect.y },
            };
            unsafe { SetCapture(hwnd) };
        }

        Hit::Bar(i) => {
            let rect = state.layout().bar_items.get(i).copied().unwrap_or_default();
            do_bar(state, hwnd, i, rect);
        }

        Hit::Sidebar(i) => on_sidebar_click(state, hwnd, i, false),

        // The chevron is the only part of a tree row that toggles it; the rest
        // of the row navigates, which is what the label invites.
        Hit::SidebarChevron(i) => on_sidebar_click(state, hwnd, i, true),

        Hit::Nav(pid, which) => {
            state.focused = pid;
            let req = match which {
                NavButton::Back => state.pane_mut(pid).go_back(),
                NavButton::Forward => state.pane_mut(pid).go_forward(),
                NavButton::Up => state.pane_mut(pid).navigate_up(),
            };
            start_load(hwnd, pid, req);
        }

        Hit::Filter(pid) => {
            state.focused = pid;
            state.filter_focus = Some(pid);
        }

        Hit::Tab(pid, i) => {
            state.focused = pid;
            let req = state.pane_mut(pid).switch_tab(i);
            start_load(hwnd, pid, req);
            // Switching happens now; moving the tab, if the mouse travels far
            // enough before it comes up, happens on the button up.
            state.tab_drag = Some((pid, i, x, y));
        }

        Hit::TabClose(pid, i) => {
            state.focused = pid;
            let req = state.pane_mut(pid).close_tab(i, FALLBACK_PATH);
            start_load(hwnd, pid, req);
        }

        Hit::NewTab(pid) => {
            state.focused = pid;
            let req = if state.pane(pid).current_path().is_empty() {
                state.pane_mut(pid).new_tab(FALLBACK_PATH)
            } else {
                state.pane_mut(pid).duplicate_tab()
            };
            spawn_dir_load(hwnd, pid, req);
        }

        Hit::Crumb(pid, segment) => {
            state.focused = pid;
            let segs = fs::path_segments(state.pane(pid).current_path());
            let path = fs::path_from_segments(&segs, segment);
            if !path.is_empty() && !pane::paths_equal(&path, state.pane(pid).current_path()) {
                let req = state.pane_mut(pid).navigate(&path);
                spawn_dir_load(hwnd, pid, req);
            }
        }

        Hit::Column(pid, key) => {
            state.focused = pid;
            state.pane_mut(pid).list_mut().apply_sort(key);
            // Sorting by hand is a statement about this folder, so coming back
            // to it later comes back to this order too.
            state.remember_current_view();
        }

        Hit::ColumnEdge(pid, key) => {
            state.focused = pid;
            if let Some(i) = AppState::col_index(key) {
                state.drag = Drag::Column {
                    key,
                    start_x: x,
                    start_w: state.col_widths[i],
                };
                unsafe { SetCapture(hwnd) };
            }
        }

        Hit::Row(pid, row) => {
            state.focused = pid;
            let already = state.pane(pid).list().is_selected(row);
            // Dragging a multi-selection must not collapse it to one row, so a
            // plain click on an already-selected row waits for the button up.
            if !(already && select_mode() == SelectMode::Replace) {
                let mode = select_mode();
                state.pane_mut(pid).list_mut().select(row, mode);
            }
            state.drag_origin = Some((x, y));
        }

        Hit::ListBackground(pid) => {
            state.focused = pid;
            let p = layout.pane(pid);
            let list = state.pane(pid).list();
            let row = p.cell_at(x, y, list.scroll_px(), list.total_rows());
            match row {
                Some(row) => {
                    let mode = select_mode();
                    state.pane_mut(pid).list_mut().select(row, mode);
                }
                None => {
                    if !ctrl_down() {
                        state.pane_mut(pid).list_mut().clear_selection();
                    }
                    // Pressing in the empty space below the rows starts a
                    // rubber band. Pressing on a row does not: there it means
                    // "drag these files", which `drag_origin` already handles.
                    state.drag = Drag::Band {
                        pid,
                        origin: (x, y),
                        cursor: (x, y),
                    };
                    unsafe { SetCapture(hwnd) };
                }
            }
        }

        Hit::ScrollbarThumb(pid) => {
            state.focused = pid;
            let thumb = layout.pane(pid).thumb;
            state.drag = Drag::Scrollbar {
                pid,
                grab_offset: y - thumb.y,
            };
            unsafe { SetCapture(hwnd) };
        }

        Hit::ScrollbarTrack(pid, t) => {
            state.focused = pid;
            let p = layout.pane(pid);
            let total = state.pane(pid).list().total_rows();
            let visible = p.visible_rows;
            let offset = (t * total.saturating_sub(visible) as f32).round() as u32;
            let h = p.list.h.max(0) as u32;
            state.pane_mut(pid).list_mut().scroll_to(offset, h);
        }

        Hit::Nothing => {}
    }
    invalidate(hwnd);
}

/// One handler for both halves of a sidebar row. `chevron` is true when the
/// click landed on the expander rather than the label.
fn on_sidebar_click(state: &mut AppState, hwnd: HWND, index: usize, chevron: bool) {
    let (_, actions) = state.sidebar_model();
    match actions.get(index) {
        Some(SidebarAction::ToggleSection(sec)) => {
            let sec = *sec;
            state.sections_collapsed[sec] = !state.sections_collapsed[sec];
            if sec == SECTION_TREE && !state.sections_collapsed[sec] {
                state.refresh_tree();
            }
        }
        Some(SidebarAction::Go(path)) => {
            let path = path.clone();
            if chevron {
                // A chevron hit on a tree row arrives here because the row's
                // action is Go; expanding is what the chevron means.
                state.tree.toggle(&path);
                state.refresh_tree();
                return;
            }
            let pid = state.focused;
            let req = state.pane_mut(pid).navigate(&path);
            spawn_dir_load(hwnd, pid, req);
        }
        None => {}
    }
}

pub fn on_double_click(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
    let layout = state.layout();
    let hit = layout.hit_test(x, y);

    // Double-clicking a divider shares the width out evenly, like a window
    // splitter should.
    if let Hit::Divider(i) = hit {
        // Even shares for this divider's own two sides, not for the window:
        // with a tree, halving one split must leave the others alone.
        state.set_divider(i, 0.5);
        invalidate(hwnd);
        return;
    }

    let pid = match hit {
        Hit::Row(s, _) | Hit::ListBackground(s) => s,
        _ => return,
    };
    state.focused = pid;
    let p = layout.pane(pid);
    let list = state.pane(pid).list();
    let Some(row) = p.cell_at(x, y, list.scroll_px(), list.total_rows()) else {
        return;
    };
    state.pane_mut(pid).list_mut().select(row, SelectMode::Replace);
    activate_selection(state, hwnd, pid);
}

/// Enter or double-click: open a folder in place, step into an archive, or
/// hand a file to the shell.
pub fn activate_selection(state: &mut AppState, hwnd: HWND, pid: PaneId) {
    let pane = state.pane(pid);
    let Some(entry) = pane.list().cursor_entry() else {
        return;
    };
    let dir = pane.current_path().to_string();
    let name = entry.name.clone();
    let is_dir = entry.is_dir;
    let target = fs::child_path(&dir, entry);

    // An archive opens like a folder. The path keeps growing through it, so
    // Back, Up and the breadcrumb work on the way out with no special case.
    if is_dir || crate::archive::is_archive_name(&name) {
        let req = state.pane_mut(pid).navigate(&target);
        spawn_dir_load(hwnd, pid, req);
        invalidate(hwnd);
        return;
    }

    // A file inside an archive has to come out before anything can open it.
    if let Some((archive, inner)) = crate::archive::split(&target) {
        state.status_override = Some("Extracting\u{2026}".into());
        match crate::archive::extract_to_temp(&archive, &inner) {
            Ok(temp) => {
                state.status_override = Some("Opened a read-only copy".into());
                if let Err(e) = ops::shell_open(hwnd, &temp) {
                    report_error(hwnd, "Cannot open file", &ops::format_hresult(&e));
                }
            }
            Err(e) => report_error(hwnd, "Cannot open file", &e),
        }
        invalidate(hwnd);
        return;
    }

    if let Err(e) = ops::shell_open(hwnd, &target) {
        report_error(hwnd, "Cannot open file", &ops::format_hresult(&e));
    }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

pub fn on_key_down(state: &mut AppState, hwnd: HWND, vk: VIRTUAL_KEY) -> bool {
    // While the footer filter has focus it owns editing keys; everything else
    // still falls through to the normal bindings.
    if let Some(fside) = state.filter_focus {
        match vk {
            VK_BACK => {
                state.pane_mut(fside).list_mut().pop_filter_char();
                invalidate(hwnd);
                return true;
            }
            VK_ESCAPE => {
                state.pane_mut(fside).list_mut().clear_filter();
                state.filter_focus = None;
                invalidate(hwnd);
                return true;
            }
            VK_RETURN | VK_DOWN | VK_UP => {
                // Commit the filter and hand the arrows back to the list.
                state.filter_focus = None;
                invalidate(hwnd);
                if vk == VK_RETURN {
                    return true;
                }
            }
            _ => {}
        }
    }

    state.status_override = None;
    let pid = state.focused;
    let list_h = state.list_height(pid);
    let ctrl = ctrl_down();
    let shift = shift_down();
    let alt = alt_down();
    let mode = select_mode();

    // Bound chords win, and they are the only thing a person can change. What
    // is left below is movement — arrows, paging, Tab, type-ahead — whose
    // meaning shifts with Ctrl and Shift rather than naming a command.
    //
    // Consulting the table first also settles a conflict the old match had:
    // `VK_LEFT if grid` came before `VK_LEFT if alt`, so Alt+Left moved
    // the cursor in the icon view instead of going back.
    let chord = crate::keys::Chord::new(vk.0, ctrl, shift, alt);
    if let Some(cmd) = state.bindings.command_for(chord) {
        run_command(state, hwnd, cmd);
        invalidate(hwnd);
        return true;
    }

    let mut handled = true;
    match vk {
        // Left and Right step one entry; Up and Down step one line, which in
        // the icon view is a whole row of them. Same flat indices either way.
        VK_LEFT | VK_RIGHT if state.icons != crate::layout::ICONS_OFF => {
            let delta = if vk == VK_LEFT { -1 } else { 1 };
            let list = state.pane_mut(pid).list_mut();
            if let Some(n) = list.move_cursor(delta, mode) {
                list.ensure_visible(n, list_h);
            }
        }

        VK_UP | VK_DOWN => {
            let step = state.pane(pid).list().columns.max(1) as i32;
            let delta = if vk == VK_UP { -step } else { step };
            if alt && vk == VK_UP {
                let req = state.pane_mut(pid).navigate_up();
                start_load(hwnd, pid, req);
            } else {
                let list = state.pane_mut(pid).list_mut();
                let next = if ctrl && !shift {
                    list.move_cursor_only(delta)
                } else {
                    list.move_cursor(delta, mode)
                };
                if let Some(n) = next {
                    list.ensure_visible(n, list_h);
                }
            }
        }

        VK_PRIOR | VK_NEXT => {
            let layout = state.layout();
            let p = layout.pane(pid);
            let page = (p.visible_rows.max(1) * p.columns_per_line.max(1)) as i32;
            let delta = if vk == VK_PRIOR { -page } else { page };
            let list = state.pane_mut(pid).list_mut();
            if let Some(n) = list.move_cursor(delta, mode) {
                list.ensure_visible(n, list_h);
            }
        }

        VK_HOME | VK_END => {
            let list = state.pane_mut(pid).list_mut();
            let target = if vk == VK_HOME {
                0
            } else {
                list.total_rows().saturating_sub(1)
            };
            if let Some(n) = list.cursor_to(target, mode) {
                list.ensure_visible(n, list_h);
            }
        }

        VK_TAB if ctrl => {
            let delta = if shift { -1 } else { 1 };
            let req = state.pane_mut(pid).next_tab(delta);
            start_load(hwnd, pid, req);
        }
        VK_TAB => {
            // Cycles rightwards through however many panes are on screen.
            state.focused = state.other_side();
        }

        // Plain Del is bound to the Delete command; this is Shift+Del, which
        // is the same operation asking to skip the Recycle Bin.
        VK_DELETE => do_delete(state, hwnd, shift),

        VK_ESCAPE => {
            if state.pane(pid).is_searching() {
                // Leave the results and go back to the folder they came from.
                state.searches[pid.0] = None;
                let path = state.pane(pid).current_path().to_string();
                let req = state.pane_mut(pid).navigate(&path);
                spawn_dir_load(hwnd, pid, req);
            } else {
                let list = state.pane_mut(pid).list_mut();
                if list.is_filtered() {
                    list.clear_filter();
                } else {
                    list.clear_selection();
                }
            }
            state.type_ahead.clear();
        }

        _ => {
            let c = vk.0 as u8 as char;
            match (ctrl, shift, c) {
                (true, false, 'A') => state.pane_mut(pid).list_mut().select_all(),
                (true, false, 'C') => do_clipboard(state, hwnd, ops::DropEffect::Copy),
                (true, false, 'X') => do_clipboard(state, hwnd, ops::DropEffect::Move),
                (true, false, 'V') => do_paste(state, hwnd),
                (true, false, 'T') => {
                    let req = if state.pane(pid).current_path().is_empty() {
                        state.pane_mut(pid).new_tab(FALLBACK_PATH)
                    } else {
                        state.pane_mut(pid).duplicate_tab()
                    };
                    spawn_dir_load(hwnd, pid, req);
                }
                (true, false, 'W') => {
                    let req = state.pane_mut(pid).close_active_tab(FALLBACK_PATH);
                    start_load(hwnd, pid, req);
                }
                (true, false, 'Z') => do_undo(state, hwnd),
                (true, false, 'L') => do_goto(state, hwnd),
                (true, false, 'H') => toggle_hidden(state, hwnd),
                (true, false, 'F') => {
                    // Ctrl+F focuses the pane filter, as it does everywhere else.
                    state.filter_focus = Some(pid);
                }
                (true, false, 'B') => state.sidebar_visible = !state.sidebar_visible,
                (true, false, '1') => set_pane_count(state, 1),
                (true, false, '2') => set_pane_count(state, 2),
                (true, false, '3') => set_pane_count(state, 3),
                (true, false, '4') => set_pane_count(state, 4),
                (true, false, 'R') => {
                    let req = state.pane_mut(pid).refresh();
                    start_load(hwnd, pid, req);
                }
                (true, true, 'F') => prompt_search(state, hwnd),
                (true, true, 'G') => prompt_search_contents(state, hwnd),
                (true, true, 'P') => show_palette(state, hwnd),
                (true, true, 'R') => do_batch_rename(state, hwnd),
                (true, true, 'N') => do_new_folder(state, hwnd),
                (true, true, 'C') => do_copy_path(state, hwnd),
                (true, true, 'M') => copy_or_move_to_other(state, hwnd, true),
                (true, true, 'D') => toggle_theme(state, hwnd),
                (true, true, 'I') => run_command(state, hwnd, CMD_GRID),
                _ => handled = false,
            }
        }
    }

    if handled {
        invalidate(hwnd);
    }
    handled
}

pub fn on_type_ahead(state: &mut AppState, hwnd: HWND, c: char) {
    // With the filter focused, typing edits the filter rather than jumping the
    // selection.
    if let Some(fside) = state.filter_focus {
        state.pane_mut(fside).list_mut().push_filter_char(c);
        invalidate(hwnd);
        return;
    }

    let now = Instant::now();
    let expired = state
        .type_ahead_at
        .map(|t| now.duration_since(t).as_millis() > TYPE_AHEAD_RESET_MS)
        .unwrap_or(true);
    if expired {
        state.type_ahead.clear();
    }
    state.type_ahead.push(c);
    state.type_ahead_at = Some(now);

    let pid = state.focused;
    let list_h = state.list_height(pid);
    let prefix = state.type_ahead.clone();
    let list = state.pane_mut(pid).list_mut();
    // A repeated single character steps through matches instead of searching
    // for a doubled prefix, which is how Explorer behaves.
    let found = list.find_prefix(&prefix).or_else(|| {
        if prefix.chars().count() > 1 && prefix.chars().all(|x| x == c) {
            list.find_prefix(&c.to_string())
        } else {
            None
        }
    });
    if let Some(i) = found {
        list.select(i, SelectMode::Replace);
        list.ensure_visible(i, list_h);
        invalidate(hwnd);
    }
}

pub fn toggle_hidden(state: &mut AppState, hwnd: HWND) {
    state.show_hidden = !state.show_hidden;
    // FileList keeps the unfiltered entries, so this is instant: no reload.
    let show = state.show_hidden;
    for p in &mut state.panes {
        p.set_show_hidden(show);
    }
    invalidate(hwnd);
}

/// Show `n` panes side by side.
pub fn set_pane_count(state: &mut AppState, n: usize) {
    state.set_pane_count(n);
}

/// Keep the title bar showing the focused folder. Called from paint, which is
/// the one place that always runs after anything that could change it.
pub fn update_title(state: &mut AppState, hwnd: HWND) {
    let path = state.focused_pane().current_path();
    let title = if path.is_empty() {
        "File Xplorer".to_string()
    } else {
        format!("{} - File Xplorer", fs::path_leaf(path))
    };
    if title == state.last_title {
        return;
    }
    let w = wide(&title);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(w.as_ptr()));
    }
    state.last_title = title;
}

pub fn toggle_theme(state: &mut AppState, hwnd: HWND) {
    set_theme(state, hwnd, state.theme.toggled());
}

/// The one way the theme changes: the renderer's brushes and the window's
/// caption both follow from it, and either one left behind is visible.
pub fn set_theme(state: &mut AppState, hwnd: HWND, theme: crate::theme::Theme) {
    state.theme = theme;
    state.renderer.set_theme(theme);
    apply_titlebar_theme(hwnd, theme);
}

