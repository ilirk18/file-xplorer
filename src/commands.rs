// Every command the app can run, and the two surfaces that reach them.
//
// One `COMMANDS` table drives both the palette and the context menu, so a
// command is described in exactly one place: adding one here makes it
// searchable and right-clickable at the same time.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::*;
use crate::file_list::SelectMode;
use crate::input::{activate_selection, set_pane_count, toggle_hidden, toggle_theme};
use crate::layout::{Hit, PaneId};
use crate::prompt::prompt_text;
use crate::{batch_rename, fs, ops, palette, pane, search, shellmenu};


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
pub const CMD_SEARCH_CONTENTS: usize = 132;
pub const CMD_EXTRACT: usize = 133;
pub const CMD_PALETTE: usize = 134;
pub const CMD_GOTO: usize = 135;
pub const CMD_TERMINAL: usize = 136;
pub const CMD_COPY_PATH: usize = 137;
pub const CMD_PIN: usize = 138;
pub const CMD_REVEAL: usize = 139;
pub const CMD_ARCHIVE: usize = 140;
pub const CMD_COMPARE_CONTENT: usize = 141;
pub const CMD_RECENT: usize = 142;
pub const CMD_INSPECTOR: usize = 143;
pub const CMD_GRID: usize = 144;
pub const CMD_BIND: usize = 145;

/// Everything the palette can reach. The context menu builds from the same
/// list, so a command is described in exactly one place.
pub struct CommandDef {
    pub id: usize,
    pub label: &'static str,
    /// The *default* shortcut. What is actually bound lives in
    /// `AppState::bindings`, which is seeded from these and may then differ.
    pub keys: &'static str,
}

pub const COMMANDS: &[CommandDef] = &[
    CommandDef { id: CMD_OPEN, label: "Open", keys: "Enter" },
    CommandDef { id: CMD_OPEN_NEW_TAB, label: "Open in new tab", keys: "" },
    CommandDef { id: CMD_CUT, label: "Cut", keys: "Ctrl+X" },
    CommandDef { id: CMD_COPY, label: "Copy", keys: "Ctrl+C" },
    CommandDef { id: CMD_PASTE, label: "Paste", keys: "Ctrl+V" },
    CommandDef { id: CMD_COPY_TO_OTHER, label: "Copy to next pane", keys: "F6" },
    CommandDef { id: CMD_MOVE_TO_OTHER, label: "Move to next pane", keys: "Ctrl+Shift+M" },
    CommandDef { id: CMD_DELETE, label: "Delete", keys: "Del" },
    CommandDef { id: CMD_RENAME, label: "Rename", keys: "F2" },
    CommandDef { id: CMD_BATCH_RENAME, label: "Batch rename", keys: "Ctrl+Shift+R" },
    CommandDef { id: CMD_NEW_FOLDER, label: "New folder", keys: "Ctrl+Shift+N" },
    CommandDef { id: CMD_PROPERTIES, label: "Properties", keys: "" },
    CommandDef { id: CMD_SEARCH, label: "Search here", keys: "Ctrl+Shift+F" },
    CommandDef { id: CMD_SEARCH_CONTENTS, label: "Search file contents", keys: "Ctrl+Shift+G" },
    CommandDef { id: CMD_FILTER, label: "Filter this pane", keys: "Ctrl+F" },
    CommandDef { id: CMD_CALC_SIZES, label: "Calculate folder sizes", keys: "" },
    CommandDef { id: CMD_EXTRACT, label: "Extract from archive", keys: "" },
    CommandDef { id: CMD_UNDO, label: "Undo last operation", keys: "Ctrl+Z" },
    CommandDef { id: CMD_REFRESH, label: "Refresh", keys: "F5" },
    CommandDef { id: CMD_SELECT_ALL, label: "Select all", keys: "Ctrl+A" },
    CommandDef { id: CMD_GOTO, label: "Go to path", keys: "Ctrl+L" },
    CommandDef { id: CMD_RECENT, label: "Recent folders", keys: "" },
    CommandDef { id: CMD_TERMINAL, label: "Open terminal here", keys: "" },
    CommandDef { id: CMD_COPY_PATH, label: "Copy path", keys: "Ctrl+Shift+C" },
    CommandDef { id: CMD_PIN, label: "Pin or unpin this folder", keys: "" },
    CommandDef { id: CMD_REVEAL, label: "Show this folder in the tree", keys: "" },
    CommandDef { id: CMD_ARCHIVE, label: "Add to archive", keys: "" },
    CommandDef { id: CMD_COMPARE_CONTENT, label: "Compare contents with next pane", keys: "" },
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
    CommandDef { id: CMD_INSPECTOR, label: "Toggle inspector", keys: "Alt+P" },
    CommandDef { id: CMD_GRID, label: "Toggle icon view", keys: "Ctrl+Shift+I" },
    CommandDef { id: CMD_BIND, label: "Change a shortcut", keys: "" },
    CommandDef { id: CMD_TOGGLE_THEME, label: "Toggle dark / light theme", keys: "Ctrl+Shift+D" },
];

/// Rename the selection from a pattern, previewed before anything happens.
pub fn do_batch_rename(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let pane = state.focused_pane();
    let dir = pane.current_path().to_string();
    // The date comes along because `{d}` needs it and only the listing has it.
    // `format_filetime` yields "YYYY-MM-DD HH:MM"; the time half carries a colon,
    // which is not legal in a file name, so only the date half is offered.
    let items: Vec<batch_rename::Item> = pane
        .list()
        .selected_entries()
        .iter()
        .map(|e| batch_rename::Item {
            name: e.name.clone(),
            date: fs::format_filetime(e.modified)
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    if items.is_empty() || dir.is_empty() {
        return;
    }

    state.modal = true;
    let pattern = batch_rename::prompt(hwnd, state.dpi, items.clone());
    state.modal = false;

    let Some(pattern) = pattern else { return };
    let plan = batch_rename::plan(&items, &pattern);
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

/// Every command's id and default shortcut, for seeding the bindings.
pub fn default_bindings() -> Vec<(usize, &'static str)> {
    COMMANDS.iter().map(|c| (c.id, c.keys)).collect()
}

pub fn label_for(id: usize) -> &'static str {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.label)
        .unwrap_or("")
}

pub fn id_for_label(label: &str) -> Option<usize> {
    COMMANDS
        .iter()
        .find(|c| c.label.eq_ignore_ascii_case(label))
        .map(|c| c.id)
}

/// Show every command, filtered as you type.
pub fn show_palette(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    // The shortcut shown is whatever is bound now, not the default baked into
    // the table: a rebound command that still advertised its old chord would
    // be worse than showing none.
    let items: Vec<palette::Item> = COMMANDS
        .iter()
        .map(|c| palette::Item {
            label: c.label.to_string(),
            detail: state.bindings.text_for(c.id),
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
pub fn start_search(state: &mut AppState, hwnd: HWND, query: search::Query) {
    let pid = state.focused;
    if state.pane(pid).current_path().is_empty() {
        return;
    }
    let (tab_id, generation, root) = state.pane_mut(pid).begin_search(&query.label());

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.searches[pid.0] = Some(search::Search {
        cancel: cancel.clone(),
    });

    let owner = ops::OwnerWindow(hwnd);
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

/// Ask for a name pattern, then search. Separate from `start_search` so the
/// palette and the keyboard can both reach it.
pub fn prompt_search(state: &mut AppState, hwnd: HWND) {
    let here = fs::path_leaf(state.focused_pane().current_path());
    let Some(pattern) = prompt_text(
        hwnd,
        state,
        "Search",
        &format!("Find in {} and below (* allowed):", here),
        "",
    ) else {
        return;
    };
    if pattern.trim().is_empty() {
        return;
    }
    start_search(state, hwnd, search::Query::by_name(pattern.trim()));
}

/// Ask for text and search inside files rather than names.
pub fn prompt_search_contents(state: &mut AppState, hwnd: HWND) {
    let here = fs::path_leaf(state.focused_pane().current_path());
    let Some(text) = prompt_text(
        hwnd,
        state,
        "Search file contents",
        &format!("Text to find in files under {}:", here),
        "",
    ) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    start_search(state, hwnd, search::Query::by_content(text.trim()));
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

/// Unpack the selection, or the whole archive when nothing is selected, into
/// the folder the archive itself lives in.
pub fn do_extract(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let here = state.pane(pid).current_path().to_string();
    let Some((archive, inner)) = crate::archive::split(&here) else {
        state.status_override = Some("Not inside an archive".into());
        return;
    };
    let Some(dest) = fs::path_parent(&archive) else {
        return;
    };

    // Paths relative to the archive root are what 7-Zip's filter wants, and
    // they are exactly what the current inner path plus the row name gives.
    let selected: Vec<String> = state
        .pane(pid)
        .list()
        .selected_entries()
        .iter()
        .map(|e| {
            if inner.is_empty() {
                e.name.clone()
            } else {
                fs::path_join(&inner, &e.name)
            }
        })
        .collect();

    state.status_override = Some("Extracting\u{2026}".into());
    match crate::archive::extract_to(&archive, &selected, &dest) {
        Ok(()) => {
            state.status_override = Some(format!("Extracted to {}", fs::path_leaf(&dest)));
            // Whichever pane is showing the destination now has more in it.
            for p in state.visible().collect::<Vec<_>>() {
                if pane::paths_equal(state.pane(p).current_path(), &dest) {
                    let req = state.pane_mut(p).refresh();
                    start_load(hwnd, p, req);
                }
            }
        }
        Err(e) => {
            state.status_override = None;
            report_error(hwnd, "Extract", &e);
        }
    }
}

/// Type a path and go there. The edit box gets the shell's own completion, so
/// this is a folder picker without a folder picker.
pub fn do_goto(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let here = state.pane(pid).current_path().to_string();
    let Some(path) = crate::prompt::prompt_path(hwnd, state, "Go to", "Path:", &here) else {
        return;
    };
    let path = path.trim().trim_matches('"');
    if path.is_empty() {
        return;
    }
    // Environment variables are how people actually write these down.
    let expanded = fs::expand_env(path);
    let req = state.pane_mut(pid).navigate(&expanded);
    spawn_dir_load(hwnd, pid, req);
}

/// Finish an inline rename: `commit` applies what was typed, otherwise the
/// name is left alone. Either way the editor goes.
///
/// A name that is unchanged, empty, or illegal is not an error worth a message
/// box here — the box is gone and the file is untouched, which is what
/// abandoning an edit looks like. An illegal one does say why, because that is
/// a typo the person meant to fix rather than a change of mind.
pub fn finish_rename(state: &mut AppState, hwnd: HWND, commit: bool) {
    let Some(editor) = state.rename.take() else {
        return;
    };
    let new_name = editor.text();
    let old_name = editor.old_name.clone();
    let dir = state.pane(editor.pid).current_path().to_string();
    drop(editor);
    invalidate(hwnd);

    if !commit || new_name.is_empty() || new_name == old_name {
        return;
    }
    if let Err(e) = fs::validate_file_name(&new_name) {
        report_error(hwnd, "Invalid name", &e.message());
        return;
    }
    spawn_op(
        hwnd,
        ops::Op::Rename {
            source: fs::path_join(&dir, &old_name),
            new_name,
        },
    );
}

/// Pick a command, then type the chord it should answer to.
///
/// ponytail: the chord is typed as text rather than captured by pressing it.
/// A capture box would be nicer and means a modal window whose whole job is to
/// swallow every key including the ones that would otherwise close it; the
/// parser has to exist either way, and this reuses the prompt already here.
pub fn do_bind(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let items: Vec<palette::Item> = COMMANDS
        .iter()
        .map(|c| palette::Item {
            label: c.label.to_string(),
            detail: state.bindings.text_for(c.id),
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, "Change a shortcut", items);
    state.modal = false;
    let Some(i) = chosen else { return };
    let id = COMMANDS[i].id;

    let current = state.bindings.text_for(id);
    let label = COMMANDS[i].label;
    let Some(typed) = prompt_text(
        hwnd,
        state,
        "Change a shortcut",
        &format!("Keys for {} (empty to unbind):", label),
        &current,
    ) else {
        return;
    };

    if typed.trim().is_empty() {
        state.bindings.clear(id);
        state.status_override = Some(format!("{} has no shortcut", label));
        return;
    }
    let Some(chord) = crate::keys::Chord::parse(&typed) else {
        report_error(
            hwnd,
            "Not a shortcut",
            &format!(
                "{} is not a key combination.\n\nWrite them like Ctrl+Shift+P, Alt+Left or F5.",
                typed
            ),
        );
        return;
    };
    // Say what it displaced, rather than letting a shortcut quietly stop working.
    let taken = state
        .bindings
        .command_for(chord)
        .filter(|other| *other != id)
        .map(label_for);
    state.bindings.set(chord, id);
    state.status_override = Some(match taken {
        Some(other) => format!("{} is now {}, taken from {}", chord.text(), label, other),
        None => format!("{} is now {}", chord.text(), label),
    });
}

/// Pick from the folders visited this session and before, most recent first.
///
/// The palette is the list widget we already have, and a recent folder is
/// exactly what it is good at: a short list, filtered by typing.
pub fn do_recent(state: &mut AppState, hwnd: HWND) {
    if state.modal || state.recent.is_empty() {
        if state.recent.is_empty() {
            state.status_override = Some("No folders visited yet".into());
        }
        return;
    }
    let paths = state.recent.clone();
    let items: Vec<palette::Item> = paths
        .iter()
        .map(|p| palette::Item {
            label: fs::path_leaf(p),
            detail: p.clone(),
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, "Recent folders", items);
    state.modal = false;

    if let Some(i) = chosen {
        let pid = state.focused;
        let req = state.pane_mut(pid).navigate(&paths[i]);
        spawn_dir_load(hwnd, pid, req);
    }
}

/// Open a shell in the focused folder, or in the selected folder if there is one.
pub fn do_terminal(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let mut dir = state.pane(pid).current_path().to_string();
    if let Some(e) = state.pane(pid).list().cursor_entry() {
        if e.is_dir && state.pane(pid).list().selection_count() == 1 {
            dir = fs::path_join(&dir, &e.name);
        }
    }
    // A path inside an archive is not a directory anything can start in.
    if dir.is_empty() || crate::archive::split(&dir).is_some() {
        state.status_override = Some("No folder to open a terminal in".into());
        return;
    }
    if let Err(e) = ops::open_terminal(hwnd, &dir) {
        report_error(hwnd, "Open terminal", &ops::format_hresult(&e));
    }
}

/// The selection's full paths as text, one per line; the folder's own path
/// when nothing is selected.
pub fn do_copy_path(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let mut paths = state.pane(pid).selected_paths();
    if paths.is_empty() {
        paths.push(state.pane(pid).current_path().to_string());
    }
    let n = paths.len();
    if let Err(e) = ops::clipboard_write_text(hwnd, &paths.join("\r\n")) {
        report_error(hwnd, "Clipboard", &ops::format_hresult(&e));
        return;
    }
    state.status_override = Some(format!("{} path{} copied", n, if n == 1 { "" } else { "s" }));
}

/// Open the sidebar tree down to the focused folder and scroll it into view.
pub fn do_reveal(state: &mut AppState, hwnd: HWND) {
    let path = state.focused_pane().current_path().to_string();
    if path.is_empty() || crate::archive::split(&path).is_some() {
        return;
    }
    let landed = state.tree.reveal(&path);
    state.sections_collapsed[SECTION_TREE] = false;
    state.refresh_tree();

    // Find where that row ended up and scroll so it is on screen. Laying the
    // sidebar out twice is cheaper than duplicating its row arithmetic here.
    let index = state
        .tree_rows
        .iter()
        .position(|r| crate::pane::paths_equal(&r.path, &landed));
    if let Some(i) = index {
        let (entries, _) = state.sidebar_model();
        let offset = entries
            .iter()
            .position(|e| matches!(e, crate::layout::SidebarEntry::Tree { index, .. } if *index == i));
        if let Some(entry_index) = offset {
            let before: i32 = entries[..entry_index]
                .iter()
                .map(|e| crate::layout::sidebar_content_h(state.metrics, std::slice::from_ref(e)) - state.metrics.pad)
                .sum();
            // Put it a third of the way down rather than at the very top, so
            // its parents stay visible.
            state.sidebar_scroll = before - state.client.h / 3;
            state.clamp_sidebar_scroll();
        }
    }
    invalidate(hwnd);
}

/// Pack the selection into a new archive beside it.
pub fn do_archive(state: &mut AppState, hwnd: HWND) {
    let pid = state.focused;
    let sources = state.pane(pid).selected_paths();
    if sources.is_empty() {
        state.status_override = Some("Nothing selected".into());
        return;
    }
    let dir = state.pane(pid).current_path().to_string();
    let suggested = format!("{}.zip", fs::path_leaf(&sources[0]));
    let Some(name) = prompt_text(hwnd, state, "Add to archive", "Archive name:", &suggested) else {
        return;
    };
    if let Err(e) = fs::validate_file_name(&name) {
        report_error(hwnd, "Invalid name", &e.message());
        return;
    }
    let target = fs::path_join(&dir, &name);
    state.status_override = Some("Archiving\u{2026}".into());
    spawn_task(hwnd, move || match crate::archive::create(&target, &sources) {
        Ok(()) => TaskDone {
            message: format!("Created {}", fs::path_leaf(&target)),
            error: None,
            refresh: vec![dir],
        },
        Err(e) => TaskDone {
            message: String::new(),
            error: Some(e),
            refresh: Vec::new(),
        },
    });
}

/// Compare this pane's files with the pane next door, by contents rather than
/// by name. Runs on a worker thread: it reads both sides of every match.
pub fn do_compare_contents(state: &mut AppState, hwnd: HWND) {
    if state.pane_count < 2 {
        state.status_override = Some("Press Ctrl+2 for a second pane".into());
        return;
    }
    let pid = state.focused;
    let other = state.other_side();
    let dir_a = state.pane(pid).current_path().to_string();
    let dir_b = state.pane(other).current_path().to_string();
    let names_b = state.pane(other).list().names();
    let names: Vec<String> = state
        .pane(pid)
        .list()
        .entries
        .iter()
        .filter(|e| !e.is_dir && names_b.contains(&e.name))
        .map(|e| e.name.clone())
        .collect();
    if names.is_empty() {
        state.status_override = Some("No files in common".into());
        return;
    }

    let tab_id = state.pane(pid).active_tab_id();
    state.status_override = Some(format!("Comparing {} files\u{2026}", names.len()));
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let differing: Vec<String> = names
            .into_iter()
            .filter(|n| fs::files_differ(&fs::path_join(&dir_a, n), &fs::path_join(&dir_b, n)))
            .collect();
        let raw = Box::into_raw(Box::new(DiffDone {
            pid,
            tab_id,
            differing,
        }));
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_DIFF_DONE,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

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
    let pid = state.focused;
    let pane = state.pane(pid);
    let Some(entry) = pane.list().cursor_entry() else {
        return;
    };
    let old_name = entry.name.clone();
    let source = fs::path_join(pane.current_path(), &old_name);

    // Type over the name where it sits. The dialog stays as the fallback for a
    // row that is not on screen — from the command palette, say, where the
    // selection may be anywhere in the listing.
    if !state.modal {
        if let Some(row) = state.pane(pid).list().cursor() {
            state.rename = crate::rename::InlineRename::begin(hwnd, state, pid, row);
            if state.rename.is_some() {
                return;
            }
        }
    }

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

    // Which row, if any, was actually under the cursor. This — not "is
    // anything selected" — decides which menu appears: right-clicking the
    // empty space below a list is a question about the folder, even when a
    // row further up is still selected.
    let row = match hit {
        Hit::ListBackground(_) | Hit::Row(..) => {
            let list = state.pane(pid).list();
            layout
                .pane(pid)
                .cell_at(client.x, client.y, list.scroll_px(), list.total_rows())
        }
        _ => None,
    };
    // Right-clicking an unselected row selects it first, as Explorer does.
    if let Some(row) = row {
        if !state.pane(pid).list().is_selected(row) {
            state.pane_mut(pid).list_mut().select(row, SelectMode::Replace);
        }
    }
    invalidate(hwnd);

    let count = state.pane(pid).list().selection_count();
    let folder = state
        .pane(pid)
        .list()
        .cursor_entry()
        .map(|e| e.is_dir)
        .unwrap_or(false);
    // Nothing writes inside an archive, so none of the commands that would be
    // refused are offered.
    let in_archive = crate::archive::split(state.pane(pid).current_path()).is_some();
    let writable = !in_archive;
    let pinned = state.is_pinned(state.pane(pid).current_path());

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut themed = crate::menu::ThemedMenu::new(state.theme, state.dpi);
        // A macro rather than a closure: a closure would hold `themed`
        // borrowed for the whole block, and the separators need it too.
        // Cloned so the menu can be built while `state` is borrowed elsewhere;
        // a few dozen entries per right-click is nothing.
        let binds = state.bindings.clone();
        // The shortcut shown is whatever is bound now. Written out beside each
        // entry it would be a second copy of the binding, free to drift from
        // the one that actually fires.
        macro_rules! add {
            ($cond:expr, $id:expr, $text:expr) => {
                if $cond {
                    themed.add(menu, $id, $text, &binds.text_for($id));
                }
            };
        }

        if row.is_some() {
            // What you clicked on.
            add!(true, CMD_OPEN, "Open");
            add!(folder && count == 1, CMD_OPEN_NEW_TAB, "Open in new tab");
            add!(folder && count == 1, CMD_TERMINAL, "Open terminal here");
            themed.separator(menu);
            add!(writable, CMD_CUT, "Cut");
            add!(true, CMD_COPY, "Copy");
            add!(true, CMD_COPY_PATH, "Copy path");
            add!(state.pane_count > 1, CMD_COPY_TO_OTHER, "Copy to next pane");
            add!(state.pane_count > 1 && writable, CMD_MOVE_TO_OTHER, "Move to next pane");
            add!(in_archive, CMD_EXTRACT, "Extract here");
            add!(writable && !in_archive, CMD_ARCHIVE, "Add to archive\u{2026}");
            themed.separator(menu);
            add!(writable && count == 1, CMD_RENAME, "Rename");
            add!(writable && count > 1, CMD_BATCH_RENAME, "Rename all");
            add!(writable, CMD_DELETE, "Delete");
            themed.separator(menu);
            add!(count == 1, CMD_PROPERTIES, "Properties");
        } else {
            // What this folder can do. Everything else lives in the palette,
            // which is one line further down and searchable.
            add!(writable, CMD_NEW_FOLDER, "New folder");
            let can_paste = writable && ops::clipboard_read(hwnd).is_some();
            add!(can_paste, CMD_PASTE, "Paste");
            add!(in_archive, CMD_EXTRACT, "Extract all");
            themed.separator(menu);
            add!(true, CMD_GOTO, "Go to path\u{2026}");
            add!(true, CMD_TERMINAL, "Open terminal here");
            add!(true, CMD_PIN, if pinned { "Unpin this folder" } else { "Pin this folder" });
            add!(true, CMD_REVEAL, "Show in tree");
            themed.separator(menu);
            add!(true, CMD_SEARCH, "Search here");
            add!(true, CMD_CALC_SIZES, "Calculate folder sizes");
            add!(true, CMD_REFRESH, "Refresh");
            themed.separator(menu);
            add!(true, CMD_PALETTE, "All commands\u{2026}");
        }
        themed.apply(menu);

        // Everything installed software registered goes below our own items,
        // and only when there is something selected for it to act on.
        let paths = if row.is_some() {
            state.pane(pid).selected_paths()
        } else {
            Vec::new()
        };
        let shell = shellmenu::append(menu, hwnd, &paths);

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
        CMD_EXTRACT => do_extract(state, hwnd),
        CMD_PALETTE => show_palette(state, hwnd),
        CMD_GOTO => do_goto(state, hwnd),
        CMD_RECENT => do_recent(state, hwnd),
        CMD_BIND => do_bind(state, hwnd),
        CMD_TERMINAL => do_terminal(state, hwnd),
        CMD_COPY_PATH => do_copy_path(state, hwnd),
        CMD_PIN => {
            let path = state.pane(pid).current_path().to_string();
            state.toggle_pin(&path);
        }
        CMD_REVEAL => do_reveal(state, hwnd),
        CMD_ARCHIVE => do_archive(state, hwnd),
        CMD_COMPARE_CONTENT => do_compare_contents(state, hwnd),
        CMD_CALC_SIZES => calculate_folder_sizes(state, hwnd),
        CMD_SEARCH => prompt_search(state, hwnd),
        CMD_SEARCH_CONTENTS => prompt_search_contents(state, hwnd),
        CMD_BATCH_RENAME => do_batch_rename(state, hwnd),
        CMD_TOGGLE_THEME => toggle_theme(state, hwnd),
        CMD_TOGGLE_HIDDEN => toggle_hidden(state, hwnd),
        CMD_TOGGLE_SIDEBAR => {
            state.sidebar_visible = !state.sidebar_visible;
            state.clamp_split();
        }
        CMD_GRID => {
            state.grid = !state.grid;
            // The cursor keeps its index, so whatever was selected stays
            // selected; only the shape it is drawn in changes. Scroll is in
            // lines and a line now holds a different number of entries, so it
            // is re-derived from the cursor rather than carried over.
            state.sync_columns();
            let pid = state.focused;
            let h = state.list_height(pid);
            if let Some(c) = state.pane(pid).list().cursor() {
                state.pane_mut(pid).list_mut().ensure_visible(c, h);
            }
        }
        CMD_INSPECTOR => {
            state.inspector = !state.inspector;
            // Hiding it drops what it was holding, which for an image is a
            // bitmap worth megabytes.
            if !state.inspector {
                state.preview = None;
                state.preview_pending = None;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_shortcut_in_the_table_is_a_real_chord() {
        // The table's `keys` text is not documentation any more, it is the
        // binding. A typo here would silently leave a command unreachable.
        for c in COMMANDS {
            if c.keys.is_empty() {
                continue;
            }
            let chord = crate::keys::Chord::parse(c.keys)
                .unwrap_or_else(|| panic!("{}: {:?} is not a chord", c.label, c.keys));
            assert_eq!(
                chord.text(),
                c.keys,
                "{}: write it the way it is displayed",
                c.label
            );
        }
    }

    #[test]
    fn no_two_commands_claim_the_same_default_chord() {
        let b = crate::keys::Bindings::from_defaults(&default_bindings());
        for c in COMMANDS {
            if c.keys.is_empty() {
                continue;
            }
            let chord = crate::keys::Chord::parse(c.keys).unwrap();
            assert_eq!(
                b.command_for(chord),
                Some(c.id),
                "{} lost {} to another command",
                c.label,
                c.keys
            );
        }
    }

    #[test]
    fn command_ids_and_labels_resolve_both_ways() {
        for c in COMMANDS {
            assert_eq!(id_for_label(c.label), Some(c.id), "{}", c.label);
            assert_eq!(label_for(c.id), c.label);
        }
    }
}

