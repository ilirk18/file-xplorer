// Every command the app can run, and the two surfaces that reach them.
//
// One `COMMANDS` table drives both the palette and the context menu, so a
// command is described in exactly one place: adding one here makes it
// searchable and right-clickable at the same time.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::file_list::SelectMode;
use crate::input::{activate_selection, set_pane_count, toggle_hidden, toggle_theme};
use crate::layout::{Hit, PaneId};
use crate::prompt::prompt_text;
use crate::{batch_rename, fs, ops, palette, search, shellmenu};


// Context menu command ids.
pub const CMD_OPEN: usize = 100;
pub const CMD_OPEN_NEW_TAB: usize = 101;
pub const CMD_CUT: usize = 102;
pub const CMD_COPY: usize = 103;
pub const CMD_PASTE: usize = 104;
pub const CMD_DELETE: usize = 105;
pub const CMD_RENAME: usize = 106;
pub const CMD_NEW_FOLDER: usize = 107;
pub const CMD_PROPERTIES: usize = 108;
pub const CMD_COPY_TO_OTHER: usize = 109;
pub const CMD_MOVE_TO_OTHER: usize = 110;
pub const CMD_REFRESH: usize = 111;
pub const CMD_CALC_SIZES: usize = 112;
pub const CMD_SYNC_SCROLL: usize = 113;
pub const CMD_COMPARE: usize = 114;
pub const CMD_SEARCH: usize = 115;
pub const CMD_BATCH_RENAME: usize = 116;
pub const CMD_TOGGLE_THEME: usize = 117;
pub const CMD_TOGGLE_HIDDEN: usize = 118;
pub const CMD_TOGGLE_SIDEBAR: usize = 119;
pub const CMD_SINGLE_PANE: usize = 120;
pub const CMD_DUAL_PANE: usize = 121;
pub const CMD_THREE_PANES: usize = 129;
pub const CMD_FOUR_PANES: usize = 130;
pub const CMD_NEW_TAB: usize = 122;
pub const CMD_CLOSE_TAB: usize = 123;
pub const CMD_SELECT_ALL: usize = 124;
pub const CMD_FILTER: usize = 125;
pub const CMD_GO_UP: usize = 126;
pub const CMD_GO_BACK: usize = 127;
pub const CMD_GO_FORWARD: usize = 128;
pub const CMD_UNDO: usize = 131;

/// Everything the palette can reach. The context menu builds from the same
/// list, so a command is described in exactly one place.
pub struct CommandDef {
    id: usize,
    label: &'static str,
    keys: &'static str,
}

pub const COMMANDS: &[CommandDef] = &[
    CommandDef { id: CMD_OPEN, label: "Open", keys: "Enter" },
    CommandDef { id: CMD_OPEN_NEW_TAB, label: "Open in new tab", keys: "" },
    CommandDef { id: CMD_CUT, label: "Cut", keys: "Ctrl+X" },
    CommandDef { id: CMD_COPY, label: "Copy", keys: "Ctrl+C" },
    CommandDef { id: CMD_PASTE, label: "Paste", keys: "Ctrl+V" },
    CommandDef { id: CMD_COPY_TO_OTHER, label: "Copy to other pane", keys: "F6" },
    CommandDef { id: CMD_MOVE_TO_OTHER, label: "Move to other pane", keys: "Ctrl+Shift+M" },
    CommandDef { id: CMD_DELETE, label: "Delete", keys: "Del" },
    CommandDef { id: CMD_RENAME, label: "Rename", keys: "F2" },
    CommandDef { id: CMD_BATCH_RENAME, label: "Batch rename", keys: "Ctrl+Shift+R" },
    CommandDef { id: CMD_NEW_FOLDER, label: "New folder", keys: "Ctrl+Shift+N" },
    CommandDef { id: CMD_PROPERTIES, label: "Properties", keys: "" },
    CommandDef { id: CMD_SEARCH, label: "Search here", keys: "Ctrl+Shift+F" },
    CommandDef { id: CMD_FILTER, label: "Filter this pane", keys: "Ctrl+F" },
    CommandDef { id: CMD_CALC_SIZES, label: "Calculate folder sizes", keys: "" },
    CommandDef { id: CMD_UNDO, label: "Undo last operation", keys: "Ctrl+Z" },
    CommandDef { id: CMD_REFRESH, label: "Refresh", keys: "F5" },
    CommandDef { id: CMD_SELECT_ALL, label: "Select all", keys: "Ctrl+A" },
    CommandDef { id: CMD_GO_UP, label: "Go up", keys: "Backspace" },
    CommandDef { id: CMD_GO_BACK, label: "Go back", keys: "Alt+Left" },
    CommandDef { id: CMD_GO_FORWARD, label: "Go forward", keys: "Alt+Right" },
    CommandDef { id: CMD_NEW_TAB, label: "New tab", keys: "Ctrl+T" },
    CommandDef { id: CMD_CLOSE_TAB, label: "Close tab", keys: "Ctrl+W" },
    CommandDef { id: CMD_SINGLE_PANE, label: "Single pane", keys: "Ctrl+1" },
    CommandDef { id: CMD_DUAL_PANE, label: "Two panes", keys: "Ctrl+2" },
    CommandDef { id: CMD_THREE_PANES, label: "Three panes", keys: "Ctrl+3" },
    CommandDef { id: CMD_FOUR_PANES, label: "Four panes", keys: "Ctrl+4" },
    CommandDef { id: CMD_SYNC_SCROLL, label: "Toggle synchronised scrolling", keys: "" },
    CommandDef { id: CMD_COMPARE, label: "Toggle compare panes", keys: "" },
    CommandDef { id: CMD_TOGGLE_HIDDEN, label: "Toggle hidden files", keys: "Ctrl+H" },
    CommandDef { id: CMD_TOGGLE_SIDEBAR, label: "Toggle sidebar", keys: "Ctrl+B" },
    CommandDef { id: CMD_TOGGLE_THEME, label: "Toggle dark / light theme", keys: "Ctrl+Shift+D" },
];

/// Rename the selection from a pattern, previewed before anything happens.
pub fn do_batch_rename(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let pane = state.focused_pane();
    let dir = pane.current_path().to_string();
    let names: Vec<String> = pane
        .list()
        .selected_entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    if names.is_empty() || dir.is_empty() {
        return;
    }

    state.modal = true;
    let pattern = batch_rename::prompt(hwnd, state.dpi, names.clone());
    state.modal = false;

    let Some(pattern) = pattern else { return };
    let plan = batch_rename::plan(&names, &pattern);
    if plan.is_empty() {
        return;
    }
    let items = plan
        .into_iter()
        .map(|(old, new)| (fs::path_join(&dir, &old), new))
        .collect();
    state.status_override = Some("Renaming\u{2026}".into());
    spawn_op(hwnd, ops::Op::RenameMany { items });
}

/// Show every command, filtered as you type.
pub fn show_palette(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let items: Vec<palette::Item> = COMMANDS
        .iter()
        .map(|c| palette::Item {
            label: c.label.to_string(),
            detail: c.keys.to_string(),
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, "Commands", items);
    state.modal = false;

    if let Some(i) = chosen {
        run_command(state, hwnd, COMMANDS[i].id);
    }
}

/// Start a recursive search in the focused pane, rooted at its current folder.
pub fn start_search(state: &mut AppState, hwnd: HWND, query: &str) {
    let pid = state.focused;
    if state.pane(pid).current_path().is_empty() {
        return;
    }
    let (tab_id, generation, root) = state.pane_mut(pid).begin_search(query);

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.searches[pid.0] = Some(search::Search {
        cancel: cancel.clone(),
    });

    let owner = ops::OwnerWindow(hwnd);
    let query = query.to_string();
    std::thread::spawn(move || {
        search::run(root, query, generation, cancel, |batch| {
            let raw = Box::into_raw(Box::new(SearchBatch {
                pid,
                tab_id,
                batch,
            }));
            let posted = unsafe {
                PostMessageW(
                    Some(owner.hwnd()),
                    WM_APP_SEARCH_BATCH,
                    WPARAM(0),
                    LPARAM(raw as isize),
                )
            };
            if posted.is_err() {
                unsafe { drop(Box::from_raw(raw)) };
            }
        });
    });
    invalidate(hwnd);
}

/// Ask for a query, then search. Separate from `start_search` so the palette
/// and the keyboard can both reach it.
pub fn prompt_search(state: &mut AppState, hwnd: HWND) {
    let here = fs::path_leaf(state.focused_pane().current_path());
    let Some(query) = prompt_text(
        hwnd,
        state,
        "Search",
        &format!("Find in {} and below (* allowed):", here),
        "",
    ) else {
        return;
    };
    if query.trim().is_empty() {
        return;
    }
    start_search(state, hwnd, query.trim());
}

/// Walk each named folder on a worker thread and post the totals back.
pub fn spawn_dir_sizes(hwnd: HWND, pid: PaneId, tab_id: u64, dir: String, names: Vec<String>) {
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let sizes = names
            .into_iter()
            .map(|n| {
                let size = fs::dir_size(&fs::path_join(&dir, &n));
                (n, size)
            })
            .collect();
        let raw = Box::into_raw(Box::new(SizesDone {
            pid,
            tab_id,
            sizes,
        }));
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_SIZES_DONE,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

/// Size the selected folders, or every folder in view when nothing is selected.
pub fn calculate_folder_sizes(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let pane = state.pane(pid);
    let dir = pane.current_path().to_string();
    if dir.is_empty() {
        return;
    }
    let selected: Vec<String> = pane
        .list()
        .selected_entries()
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.clone())
        .collect();
    let names = if selected.is_empty() {
        pane.list()
            .entries
            .iter()
            .filter(|e| e.is_dir && !e.is_reparse)
            .map(|e| e.name.clone())
            .collect()
    } else {
        selected
    };
    if names.is_empty() {
        return;
    }
    state.status_override = Some(format!("Sizing {} folders\u{2026}", names.len()));
    let tab_id = state.pane(pid).active_tab_id();
    spawn_dir_sizes(hwnd, pid, tab_id, dir, names);
}


// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

/// Put back whatever the last undoable operation did.
pub fn do_undo(state: &mut AppState, hwnd: HWND) {
    let Some(op) = state.undo_stack.pop() else {
        state.status_override = Some("Nothing to undo".into());
        return;
    };
    state.status_override = Some(op.progress_text().to_string());
    spawn_op_tagged(hwnd, op, OP_WAS_UNDO);
}

pub fn do_clipboard(state: &mut AppState, hwnd: HWND, effect: ops::DropEffect) {
    let paths = state.focused_pane().selected_paths();
    if paths.is_empty() {
        return;
    }
    let n = paths.len();
    if let Err(e) = ops::clipboard_write(hwnd, &paths, effect) {
        report_error(hwnd, "Clipboard", &ops::format_hresult(&e));
        return;
    }
    state.status_override = Some(format!(
        "{} item{} {}",
        n,
        if n == 1 { "" } else { "s" },
        if effect == ops::DropEffect::Move { "cut" } else { "copied" }
    ));
}

pub fn do_paste(state: &mut AppState, hwnd: HWND) {
    let Some((sources, effect)) = ops::clipboard_read(hwnd) else {
        return;
    };
    let dest = state.focused_pane().current_path().to_string();
    if dest.is_empty() {
        return;
    }
    let op = match effect {
        ops::DropEffect::Copy => ops::Op::Copy {
            sources,
            dest_dir: dest,
        },
        ops::DropEffect::Move => ops::Op::Move {
            sources,
            dest_dir: dest,
        },
    };
    state.status_override = Some("Pasting\u{2026}".into());
    spawn_op(hwnd, op);
}

pub fn copy_or_move_to_other(state: &mut AppState, hwnd: HWND, move_it: bool) {
    if state.pane_count < 2 {
        state.status_override = Some("Press Ctrl+2 for a second pane".into());
        return;
    }
    let sources = state.focused_pane().selected_paths();
    if sources.is_empty() {
        return;
    }
    let dest = state.pane(state.other_side()).current_path().to_string();
    if dest.is_empty() {
        return;
    }
    let op = if move_it {
        ops::Op::Move {
            sources,
            dest_dir: dest,
        }
    } else {
        ops::Op::Copy {
            sources,
            dest_dir: dest,
        }
    };
    state.status_override = Some(if move_it { "Moving\u{2026}".into() } else { "Copying\u{2026}".to_string() });
    spawn_op(hwnd, op);
}

pub fn do_delete(state: &mut AppState, hwnd: HWND, permanent: bool) {
    let sources = state.focused_pane().selected_paths();
    if sources.is_empty() {
        return;
    }
    // No confirmation prompt of our own: IFileOperation shows the standard
    // Recycle Bin or permanent-delete confirmation, which is the one people
    // recognise and which correctly describes what will happen.
    state.status_override = Some("Deleting\u{2026}".into());
    spawn_op(hwnd, ops::Op::Delete { sources, permanent });
}

pub fn do_rename(state: &mut AppState, hwnd: HWND) {
    let pane = state.focused_pane();
    let Some(entry) = pane.list().cursor_entry() else {
        return;
    };
    let old_name = entry.name.clone();
    let source = fs::path_join(pane.current_path(), &old_name);

    let Some(new_name) = prompt_text(hwnd, state, "Rename", "New name:", &old_name) else {
        return;
    };
    if new_name == old_name {
        return;
    }
    if let Err(e) = fs::validate_file_name(&new_name) {
        report_error(hwnd, "Invalid name", &e.message());
        return;
    }
    spawn_op(hwnd, ops::Op::Rename { source, new_name });
}

pub fn do_new_folder(state: &mut AppState, hwnd: HWND) {
    let parent = state.focused_pane().current_path().to_string();
    if parent.is_empty() {
        return;
    }
    let Some(name) = prompt_text(hwnd, state, "New folder", "Folder name:", "New folder") else {
        return;
    };
    if let Err(e) = fs::validate_file_name(&name) {
        report_error(hwnd, "Invalid name", &e.message());
        return;
    }
    spawn_op(hwnd, ops::Op::NewFolder { parent, name });
}


// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

pub fn show_context_menu(state: &mut AppState, hwnd: HWND, screen_x: i32, screen_y: i32) {
    let mut pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    // Shift+F10 sends (-1, -1); anchor to the cursor row instead.
    if screen_x == -1 && screen_y == -1 {
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
    }
    let mut client = pt;
    unsafe {
        let _ = ScreenToClient(hwnd, &mut client);
    }

    let layout = state.layout();
    let hit = layout.hit_test(client.x, client.y);
    let pid = match hit {
        Hit::Row(s, _) | Hit::ListBackground(s) | Hit::Column(s, _) => s,
        _ => state.focused,
    };
    state.focused = pid;

    // Right-clicking an unselected row selects it first, as Explorer does.
    if let Hit::ListBackground(_) | Hit::Row(..) = hit {
        let p = layout.pane(pid);
        let list = state.pane(pid).list();
        if let Some(row) = p.row_at(
            client.y,
            list.scroll_offset,
            layout.metrics.row_h,
            list.total_rows(),
        ) {
            if !state.pane(pid).list().is_selected(row) {
                state.pane_mut(pid).list_mut().select(row, SelectMode::Replace);
            }
        }
    }
    invalidate(hwnd);

    let has_selection = state.pane(pid).list().selection_count() > 0;
    let single = state.pane(pid).list().selection_count() == 1;
    let can_paste = ops::clipboard_read(hwnd).is_some();

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |id: usize, text: &str, enabled: bool| {
            let t = wide(text);
            let flags = if enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
            let _ = AppendMenuW(menu, flags, id, PCWSTR::from_raw(t.as_ptr()));
        };
        let sep = || {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        };

        add(CMD_OPEN, "Open\tEnter", single);
        add(CMD_OPEN_NEW_TAB, "Open in new tab", single);
        sep();
        add(CMD_CUT, "Cut\tCtrl+X", has_selection);
        add(CMD_COPY, "Copy\tCtrl+C", has_selection);
        add(CMD_PASTE, "Paste\tCtrl+V", can_paste);
        sep();
        add(CMD_COPY_TO_OTHER, "Copy to other pane\tF6", has_selection);
        add(CMD_MOVE_TO_OTHER, "Move to other pane\tCtrl+Shift+M", has_selection);
        sep();
        add(CMD_DELETE, "Delete\tDel", has_selection);
        add(CMD_RENAME, "Rename\tF2", single);
        sep();
        add(CMD_NEW_FOLDER, "New folder\tCtrl+Shift+N", true);
        add(CMD_REFRESH, "Refresh\tF5", true);
        add(CMD_CALC_SIZES, "Calculate folder sizes", true);
        add(CMD_SEARCH, "Search here\tCtrl+Shift+F", true);
        add(CMD_BATCH_RENAME, "Batch rename\tCtrl+Shift+R", has_selection);
        sep();
        add(CMD_PROPERTIES, "Properties", single);

        // Everything installed software registered goes below our own items.
        let paths = state.pane(pid).selected_paths();
        let shell = shellmenu::append(menu, hwnd, &paths);
        if shell.is_some() {
            sep();
        }

        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        let id = cmd.0 as u32;
        if id >= shellmenu::SHELL_ID_FIRST {
            if let Some(shell) = &shell {
                shell.invoke(hwnd, id);
            }
            // The shell just changed something; find out what.
            let req = state.pane_mut(pid).refresh();
            start_load(hwnd, pid, req);
        } else {
            run_command(state, hwnd, cmd.0 as usize);
        }
    }
}

pub fn run_command(state: &mut AppState, hwnd: HWND, cmd: usize) {
    let pid = state.focused;
    match cmd {
        CMD_OPEN => activate_selection(state, hwnd, pid),
        CMD_OPEN_NEW_TAB => {
            let pane = state.pane(pid);
            if let Some(entry) = pane.list().cursor_entry() {
                if entry.is_dir {
                    let path = fs::path_join(pane.current_path(), &entry.name);
                    let req = state.pane_mut(pid).new_tab(&path);
                    spawn_dir_load(hwnd, pid, req);
                }
            }
        }
        CMD_CUT => do_clipboard(state, hwnd, ops::DropEffect::Move),
        CMD_COPY => do_clipboard(state, hwnd, ops::DropEffect::Copy),
        CMD_PASTE => do_paste(state, hwnd),
        CMD_COPY_TO_OTHER => copy_or_move_to_other(state, hwnd, false),
        CMD_MOVE_TO_OTHER => copy_or_move_to_other(state, hwnd, true),
        CMD_DELETE => do_delete(state, hwnd, shift_down()),
        CMD_RENAME => do_rename(state, hwnd),
        CMD_NEW_FOLDER => do_new_folder(state, hwnd),
        CMD_REFRESH => {
            let req = state.pane_mut(pid).refresh();
            start_load(hwnd, pid, req);
        }
        CMD_UNDO => do_undo(state, hwnd),
        CMD_CALC_SIZES => calculate_folder_sizes(state, hwnd),
        CMD_SEARCH => prompt_search(state, hwnd),
        CMD_BATCH_RENAME => do_batch_rename(state, hwnd),
        CMD_TOGGLE_THEME => toggle_theme(state, hwnd),
        CMD_TOGGLE_HIDDEN => toggle_hidden(state, hwnd),
        CMD_TOGGLE_SIDEBAR => {
            state.sidebar_visible = !state.sidebar_visible;
            state.clamp_split();
        }
        CMD_SINGLE_PANE => set_pane_count(state, 1),
        CMD_DUAL_PANE => set_pane_count(state, 2),
        CMD_THREE_PANES => set_pane_count(state, 3),
        CMD_FOUR_PANES => set_pane_count(state, 4),
        CMD_NEW_TAB => {
            let req = if state.pane(pid).current_path().is_empty() {
                state.pane_mut(pid).new_tab(FALLBACK_PATH)
            } else {
                state.pane_mut(pid).duplicate_tab()
            };
            spawn_dir_load(hwnd, pid, req);
        }
        CMD_CLOSE_TAB => {
            let req = state.pane_mut(pid).close_active_tab(FALLBACK_PATH);
            start_load(hwnd, pid, req);
        }
        CMD_SELECT_ALL => state.pane_mut(pid).list_mut().select_all(),
        CMD_FILTER => state.filter_focus = Some(pid),
        CMD_GO_UP => {
            let req = state.pane_mut(pid).navigate_up();
            start_load(hwnd, pid, req);
        }
        CMD_GO_BACK => {
            let req = state.pane_mut(pid).go_back();
            start_load(hwnd, pid, req);
        }
        CMD_GO_FORWARD => {
            let req = state.pane_mut(pid).go_forward();
            start_load(hwnd, pid, req);
        }
        CMD_SYNC_SCROLL => {
            state.sync_scroll = !state.sync_scroll;
            state.mirror_scroll(pid);
        }
        CMD_COMPARE => state.compare = !state.compare,
        CMD_PROPERTIES => {
            let paths = state.pane(pid).selected_paths();
            if let Some(p) = paths.first() {
                if let Err(e) = ops::shell_properties(hwnd, p) {
                    report_error(hwnd, "Properties", &ops::format_hresult(&e));
                }
            }
        }
        _ => return,
    }
    invalidate(hwnd);
}

