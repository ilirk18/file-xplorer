// Every command the app can run, and the two surfaces that reach them.
//
// One `COMMANDS` table drives both the palette and the context menu, so a
// command is described in exactly one place: adding one here makes it
// searchable and right-clickable at the same time.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
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
pub const CMD_NEW_WINDOW: usize = 146;
pub const CMD_TOGGLE_BAR: usize = 147;
pub const CMD_SPLIT_RIGHT: usize = 148;
pub const CMD_SPLIT_DOWN: usize = 149;
pub const CMD_CLOSE_PANE: usize = 150;
pub const CMD_ACTIONS: usize = 151;
pub const CMD_PIN_ACTION: usize = 152;
pub const CMD_SHARE: usize = 153;
pub const CMD_DEFAULT_APP: usize = 154;
pub const CMD_THEME: usize = 155;
pub const CMD_FONT: usize = 156;
pub const CMD_FONT_SIZE: usize = 157;
pub const CMD_DENSITY: usize = 158;
pub const CMD_MAP_DRIVE: usize = 159;
pub const CMD_DISCONNECT_DRIVE: usize = 160;
pub const CMD_FOLDER_SIZES: usize = 161;
pub const CMD_SETTINGS: usize = 162;
pub const CMD_SAVE_LAYOUT: usize = 163;
pub const CMD_LAYOUTS: usize = 164;

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
    CommandDef { id: CMD_ACTIONS, label: "Search shell actions", keys: "Ctrl+Shift+A" },
    CommandDef { id: CMD_PIN_ACTION, label: "Pin or unpin a shell action", keys: "" },
    CommandDef { id: CMD_SHARE, label: "Share", keys: "" },
    CommandDef { id: CMD_DEFAULT_APP, label: "Default file manager\u{2026}", keys: "" },
    CommandDef { id: CMD_SEARCH, label: "Search here", keys: "Ctrl+Shift+F" },
    CommandDef { id: CMD_SEARCH_CONTENTS, label: "Search file contents", keys: "Ctrl+Shift+G" },
    CommandDef { id: CMD_FILTER, label: "Filter this pane", keys: "Ctrl+F" },
    CommandDef { id: CMD_CALC_SIZES, label: "Calculate folder sizes", keys: "" },
    CommandDef { id: CMD_EXTRACT, label: "Extract from archive", keys: "" },
    CommandDef { id: CMD_UNDO, label: "Undo last operation", keys: "Ctrl+Z" },
    CommandDef { id: CMD_REFRESH, label: "Refresh", keys: "F5" },
    CommandDef { id: CMD_SELECT_ALL, label: "Select all", keys: "Ctrl+A" },
    CommandDef { id: CMD_GOTO, label: "Go to path", keys: "Ctrl+L" },
    CommandDef { id: CMD_RECENT, label: "Go to a known folder", keys: "Ctrl+Shift+L" },
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
    CommandDef { id: CMD_NEW_WINDOW, label: "New window", keys: "Ctrl+N" },
    // In the table so its shortcut is real and rebindable like every other.
    // `show_palette` leaves it out of its own list, because a palette offering
    // to open the palette is a joke that stops being funny immediately.
    CommandDef { id: CMD_PALETTE, label: "All commands", keys: "Ctrl+Shift+P" },
    CommandDef { id: CMD_CLOSE_TAB, label: "Close tab", keys: "Ctrl+W" },
    CommandDef { id: CMD_SINGLE_PANE, label: "Single pane", keys: "Ctrl+1" },
    CommandDef { id: CMD_DUAL_PANE, label: "Two panes", keys: "Ctrl+2" },
    CommandDef { id: CMD_THREE_PANES, label: "Three panes", keys: "Ctrl+3" },
    CommandDef { id: CMD_FOUR_PANES, label: "Four panes", keys: "Ctrl+4" },
    CommandDef { id: CMD_SPLIT_RIGHT, label: "Split right", keys: "Ctrl+Shift+E" },
    CommandDef { id: CMD_SPLIT_DOWN, label: "Split down", keys: "Ctrl+Shift+O" },
    CommandDef { id: CMD_CLOSE_PANE, label: "Close pane", keys: "Ctrl+Shift+W" },
    CommandDef { id: CMD_SYNC_SCROLL, label: "Toggle synchronised scrolling", keys: "" },
    CommandDef { id: CMD_COMPARE, label: "Toggle compare panes", keys: "" },
    CommandDef { id: CMD_TOGGLE_HIDDEN, label: "Toggle hidden files", keys: "Ctrl+H" },
    CommandDef { id: CMD_TOGGLE_SIDEBAR, label: "Toggle sidebar", keys: "Ctrl+B" },
    CommandDef { id: CMD_INSPECTOR, label: "Toggle inspector", keys: "Alt+P" },
    CommandDef { id: CMD_TOGGLE_BAR, label: "Toggle command bar", keys: "" },
    CommandDef { id: CMD_GRID, label: "Toggle icon view", keys: "Ctrl+Shift+I" },
    CommandDef { id: CMD_BIND, label: "Change a shortcut", keys: "" },
    CommandDef { id: CMD_TOGGLE_THEME, label: "Toggle dark / light theme", keys: "Ctrl+Shift+D" },
    CommandDef { id: CMD_THEME, label: "Choose a theme\u{2026}", keys: "" },
    CommandDef { id: CMD_FONT, label: "Choose a font\u{2026}", keys: "" },
    CommandDef { id: CMD_FONT_SIZE, label: "Text size\u{2026}", keys: "" },
    CommandDef { id: CMD_DENSITY, label: "Row density\u{2026}", keys: "" },
    CommandDef { id: CMD_MAP_DRIVE, label: "Map a network drive\u{2026}", keys: "" },
    CommandDef { id: CMD_DISCONNECT_DRIVE, label: "Disconnect a network drive\u{2026}", keys: "" },
    CommandDef { id: CMD_FOLDER_SIZES, label: "Toggle automatic folder sizes", keys: "" },
    CommandDef { id: CMD_SETTINGS, label: "Settings", keys: "Ctrl+Comma" },
    CommandDef { id: CMD_SAVE_LAYOUT, label: "Save this layout\u{2026}", keys: "" },
    CommandDef { id: CMD_LAYOUTS, label: "Switch to a saved layout\u{2026}", keys: "" },
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

// ---------------------------------------------------------------------------
// Command bar
// ---------------------------------------------------------------------------

/// What one button on the command bar does when clicked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BarAction {
    /// Run a command, the same one the palette and the context menu run.
    Run(usize),
    /// Drop a menu down under the button.
    Menu(BarMenu),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BarMenu {
    New,
    Sort,
    View,
    /// Every setting, plus whatever did not fit on the bar.
    Settings,
}

pub struct BarItem {
    pub glyph: &'static str,
    /// Empty for an icon-only button, which is most of them.
    pub label: &'static str,
    pub action: BarAction,
    pub right: bool,
}

/// The bar, left to right.
///
/// Every entry runs something that already existed. The bar is a second way to
/// reach the commands, never a second implementation of them, which is why
/// nothing below this line knows how to copy a file.
pub const BAR: &[BarItem] = &[
    BarItem { glyph: crate::renderer::glyph::ADD, label: "New", action: BarAction::Menu(BarMenu::New), right: false },
    BarItem { glyph: crate::renderer::glyph::CUT, label: "", action: BarAction::Run(CMD_CUT), right: false },
    BarItem { glyph: crate::renderer::glyph::COPY, label: "", action: BarAction::Run(CMD_COPY), right: false },
    BarItem { glyph: crate::renderer::glyph::PASTE, label: "", action: BarAction::Run(CMD_PASTE), right: false },
    BarItem { glyph: crate::renderer::glyph::RENAME, label: "", action: BarAction::Run(CMD_RENAME), right: false },
    BarItem { glyph: crate::renderer::glyph::DELETE, label: "", action: BarAction::Run(CMD_DELETE), right: false },
    BarItem { glyph: crate::renderer::glyph::SHARE, label: "", action: BarAction::Run(CMD_SHARE), right: false },
    BarItem { glyph: crate::renderer::glyph::SORT, label: "Sort", action: BarAction::Menu(BarMenu::Sort), right: false },
    BarItem { glyph: crate::renderer::glyph::VIEW, label: "View", action: BarAction::Menu(BarMenu::View), right: false },
    BarItem { glyph: crate::renderer::glyph::SETTINGS, label: "", action: BarAction::Menu(BarMenu::Settings), right: true },
    BarItem { glyph: crate::renderer::glyph::PANE_CLOSED, label: "Details", action: BarAction::Run(CMD_INSPECTOR), right: true },
];

/// What a button is called when it is not being drawn: in the overflow menu,
/// and in anything else that has to say what it does in words.
pub fn bar_name(item: &BarItem) -> &'static str {
    match item.action {
        BarAction::Run(cmd) => label_for(cmd),
        // The only button that draws no name at all.
        BarAction::Menu(BarMenu::Settings) => "Settings",
        // Every other menu button carries its name on the bar already.
        BarAction::Menu(_) => item.label,
    }
}

/// Whether a button shows something that is currently on.
///
/// Only toggles can be on, and the bar has one; the table carries the off
/// glyph, so this is also where the on one is chosen.
pub fn bar_state(state: &AppState, action: BarAction) -> (bool, &'static str) {
    match action {
        BarAction::Run(CMD_INSPECTOR) if state.inspector => {
            (true, crate::renderer::glyph::PANE_OPEN)
        }
        _ => (false, ""),
    }
}

/// Whether a button is live.
///
/// The same question the context menu asks before offering an entry, so the bar
/// cannot offer what an operation would then refuse.
pub fn bar_enabled(state: &AppState, action: BarAction) -> bool {
    let pane = state.focused_pane();
    let here = pane.current_path();
    let selected = pane.list().selection_count();
    let writable = !crate::shellns::is_shell_path(here) && crate::archive::split(here).is_none();
    match action {
        BarAction::Run(CMD_CUT) => selected > 0 && writable,
        BarAction::Run(CMD_COPY) => selected > 0,
        BarAction::Run(CMD_PASTE) => writable && ops::clipboard_has_files(),
        BarAction::Run(CMD_RENAME) => selected == 1 && writable,
        BarAction::Run(CMD_DELETE) => selected > 0 && writable,
        BarAction::Run(CMD_SHARE) => selected > 0,
        BarAction::Menu(BarMenu::New) => writable,
        _ => true,
    }
}

/// Click a command-bar button. `rect` is where it is, so a menu drops under it.
pub fn do_bar(state: &mut AppState, hwnd: HWND, index: usize, rect: crate::layout::Rect) {
    let Some(item) = BAR.get(index) else { return };
    if !bar_enabled(state, item.action) {
        return;
    }
    match item.action {
        BarAction::Run(cmd) => run_command(state, hwnd, cmd),
        BarAction::Menu(which) => bar_menu(state, hwnd, which, rect),
    }
}

/// Ids local to a bar menu, well clear of the command ids.
const SORT_FIRST: usize = 9000;
const ORDER_ASC: usize = 9100;
const ORDER_DESC: usize = 9101;
const ICON_FIRST: usize = 9200;
/// The two views that are a size *and* a side, so they are not in that run.
const ICON_LIST: usize = 9290;
const ICON_TILES: usize = 9291;
/// `BAR_FIRST + index` reopens a bar button from the settings menu, which is
/// how a hidden *Sort* still drops its own menu rather than doing nothing.
const BAR_FIRST: usize = 9300;
/// The settings menu's own choices. Each block is `FIRST + index` into the
/// table it comes from, and they are claimed in descending order below so no
/// range can swallow another.
const THEME_FIRST: usize = 9400;
const TEXT_FIRST: usize = 9500;
const DENSITY_FIRST: usize = 9600;
/// The menu inside a multi-row rename. Its own block, claimed before any
/// other because it is the highest.
const RENAME_FIRST: usize = 9700;

/// Drop a menu under a bar button, using the same themed menu as the context
/// menu so it looks like part of the app rather than part of Windows.
fn bar_menu(state: &mut AppState, hwnd: HWND, which: BarMenu, rect: crate::layout::Rect) {
    use crate::file_list::{SortKey, SortOrder};
    use crate::layout::{ICONS_OFF, ICON_STEPS};

    let pid = state.focused;
    // Which buttons did not fit. The layout already answered this: anything it
    // could not place got an empty rectangle and was never drawn.
    let hidden: Vec<usize> = if which == BarMenu::Settings {
        let placed = state.layout().bar_items;
        BAR.iter()
            .enumerate()
            .filter(|(i, it)| {
                !it.right
                    && placed.get(*i).map(|r| r.is_empty()).unwrap_or(true)
                    && bar_enabled(state, it.action)
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        Vec::new()
    };
    let list = state.pane(pid).list();
    let (key, order) = (list.sort_key, list.sort_order);
    let icons = state.icons;
    let theme = state.theme;
    let font = state.font.clone();
    let font_size = state.font_size;
    let density = state.density;
    let binds = state.bindings.clone();
    // A bullet rather than a checkmark column: the themed menu draws one string
    // per row, and three spaces keep the unticked entries lined up with it.
    let tick = |on: bool| if on { "\u{2022} " } else { "   " };

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut themed = crate::menu::ThemedMenu::new(state.theme, state.dpi);
        match which {
            BarMenu::New => {
                themed.add(menu, CMD_NEW_FOLDER, "New folder", &binds.text_for(CMD_NEW_FOLDER));
                themed.add(menu, CMD_NEW_TAB, "New tab", &binds.text_for(CMD_NEW_TAB));
                themed.add(menu, CMD_NEW_WINDOW, "New window", &binds.text_for(CMD_NEW_WINDOW));
                themed.separator(menu);
                themed.add(menu, CMD_ARCHIVE, "Add to archive\u{2026}", "");
            }
            BarMenu::Sort => {
                for (i, (k, name)) in [
                    (SortKey::Name, "Name"),
                    (SortKey::Type, "Type"),
                    (SortKey::Size, "Size"),
                    (SortKey::Date, "Date modified"),
                ]
                .iter()
                .enumerate()
                {
                    themed.add(menu, SORT_FIRST + i, &format!("{}{}", tick(*k == key), name), "");
                }
                themed.separator(menu);
                let asc = order == SortOrder::Asc;
                themed.add(menu, ORDER_ASC, &format!("{}Ascending", tick(asc)), "");
                themed.add(menu, ORDER_DESC, &format!("{}Descending", tick(!asc)), "");
            }
            BarMenu::View => {
                themed.add(
                    menu,
                    ICON_FIRST,
                    &format!("{}Details", tick(icons == ICONS_OFF)),
                    &binds.text_for(CMD_GRID),
                );
                // List and Tiles are the same grid with the name beside the
                // icon, so they tick for any size on that side of zero.
                themed.add(
                    menu,
                    ICON_LIST,
                    &format!("{}List", tick(crate::layout::beside(icons) && icons > -48)),
                    "",
                );
                themed.add(
                    menu,
                    ICON_TILES,
                    &format!("{}Tiles", tick(crate::layout::beside(icons) && icons <= -48)),
                    "",
                );
                themed.separator(menu);
                for (i, size) in ICON_STEPS.iter().enumerate() {
                    themed.add(
                        menu,
                        ICON_FIRST + 1 + i,
                        &format!("{}Icons, {} px", tick(icons == *size), size),
                        "",
                    );
                }
            }
            BarMenu::Settings => {
                // What did not fit on the bar comes first, because it is the
                // thing the window is currently hiding rather than a setting.
                for i in &hidden {
                    let it = &BAR[*i];
                    let accel = match it.action {
                        BarAction::Run(cmd) => binds.text_for(cmd),
                        BarAction::Menu(_) => String::new(),
                    };
                    themed.add(menu, BAR_FIRST + i, bar_name(it), &accel);
                }
                if !hidden.is_empty() {
                    themed.separator(menu);
                }

                // Appearance. Three lists of named choices, so three
                // submenus with a bullet against the one in force.
                if let Some(sub) = themed.submenu(menu, "Theme") {
                    for (i, t) in crate::theme::THEMES.iter().enumerate() {
                        themed.add(
                            sub,
                            THEME_FIRST + i,
                            &format!("{}{}", tick(i == theme.index()), t.name),
                            if t.dark { "Dark" } else { "Light" },
                        );
                    }
                }
                if let Some(sub) = themed.submenu(menu, "Text size") {
                    for (i, pct) in crate::layout::FONT_STEPS.iter().enumerate() {
                        themed.add(
                            sub,
                            TEXT_FIRST + i,
                            &format!("{}{}%", tick(*pct == font_size), pct),
                            if *pct == 100 { "Default" } else { "" },
                        );
                    }
                }
                if let Some(sub) = themed.submenu(menu, "Row density") {
                    for (i, (name, pct)) in crate::layout::DENSITY_STEPS.iter().enumerate() {
                        themed.add(
                            sub,
                            DENSITY_FIRST + i,
                            &format!("{}{}", tick(*pct == density), name),
                            &format!("{}%", pct),
                        );
                    }
                }
                themed.add(menu, CMD_FONT, &format!("Font: {}\u{2026}", font), "");
                themed.separator(menu);

                // What is on screen.
                let check = |on: bool| if on { "\u{2713} " } else { "   " };
                themed.add(
                    menu,
                    CMD_TOGGLE_SIDEBAR,
                    &format!("{}Sidebar", check(state.sidebar_visible)),
                    &binds.text_for(CMD_TOGGLE_SIDEBAR),
                );
                themed.add(
                    menu,
                    CMD_INSPECTOR,
                    &format!("{}Details panel", check(state.inspector)),
                    &binds.text_for(CMD_INSPECTOR),
                );
                themed.add(
                    menu,
                    CMD_TOGGLE_BAR,
                    &format!("{}Command bar", check(state.command_bar)),
                    &binds.text_for(CMD_TOGGLE_BAR),
                );
                themed.add(
                    menu,
                    CMD_TOGGLE_HIDDEN,
                    &format!("{}Hidden files", check(state.show_hidden)),
                    &binds.text_for(CMD_TOGGLE_HIDDEN),
                );
                themed.separator(menu);

                // How it behaves.
                themed.add(
                    menu,
                    CMD_SYNC_SCROLL,
                    &format!("{}Synchronised scrolling", check(state.sync_scroll)),
                    &binds.text_for(CMD_SYNC_SCROLL),
                );
                themed.add(
                    menu,
                    CMD_COMPARE,
                    &format!("{}Compare panes", check(state.compare)),
                    &binds.text_for(CMD_COMPARE),
                );
                themed.add(
                    menu,
                    CMD_FOLDER_SIZES,
                    &format!("{}Folder sizes on opening", check(state.folder_sizes)),
                    "",
                );
                themed.separator(menu);

                themed.add(menu, CMD_BIND, "Change a shortcut\u{2026}", "");
                themed.add(menu, CMD_DEFAULT_APP, "Default file manager\u{2026}", "");
                themed.add(
                    menu,
                    CMD_PALETTE,
                    "All commands\u{2026}",
                    &binds.text_for(CMD_PALETTE),
                );
            }
        }
        themed.apply(menu);

        // A menu hangs from the edge its button is anchored to, or the one
        // on the right opens off the side of the window.
        let from_right = BAR
            .iter()
            .any(|b| b.action == BarAction::Menu(which) && b.right);

        // TrackPopupMenu wants screen coordinates. The button rect is client
        // space, the same units the layout uses everywhere else — converting
        // here is what the context menu already does for its anchor.
        let mut pt = POINT {
            x: if from_right { rect.right() } else { rect.x },
            y: rect.bottom(),
        };
        let _ = ClientToScreen(hwnd, &mut pt);

        let align = if from_right {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTBUTTON | TPM_NONOTIFY | align,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        let id = cmd.0 as usize;
        if id == 0 {
            return;
        }
        if id >= DENSITY_FIRST {
            if let Some((_, pct)) = crate::layout::DENSITY_STEPS.get(id - DENSITY_FIRST) {
                state.density = *pct;
                apply_font(state, hwnd);
            }
        } else if id >= TEXT_FIRST {
            if let Some(pct) = crate::layout::FONT_STEPS.get(id - TEXT_FIRST) {
                state.font_size = *pct;
                apply_font(state, hwnd);
            }
        } else if id >= THEME_FIRST {
            let want = crate::theme::Theme::at(id - THEME_FIRST);
            crate::input::set_theme(state, hwnd, want);
        } else if id >= BAR_FIRST {
            // A button that did not fit, run from the overflow menu. Menus
            // reopen under the overflow button, which is where the click was.
            do_bar(state, hwnd, id - BAR_FIRST, rect);
        } else if id == ICON_LIST || id == ICON_TILES {
            state.icons = if id == ICON_LIST {
                crate::layout::LIST
            } else {
                crate::layout::TILES
            };
            state.remember_current_view();
            state.sync_columns();
        } else if id >= ICON_FIRST {
            let step = id - ICON_FIRST;
            state.icons = if step == 0 {
                ICONS_OFF
            } else {
                ICON_STEPS[(step - 1).min(ICON_STEPS.len() - 1)]
            };
            state.remember_current_view();
            state.sync_columns();
        } else if id == ORDER_ASC || id == ORDER_DESC {
            let want = if id == ORDER_ASC { SortOrder::Asc } else { SortOrder::Desc };
            state.pane_mut(pid).list_mut().set_sort(key, want);
            state.remember_current_view();
        } else if id >= SORT_FIRST {
            let want =
                [SortKey::Name, SortKey::Type, SortKey::Size, SortKey::Date][(id - SORT_FIRST).min(3)];
            state.pane_mut(pid).list_mut().set_sort(want, order);
            state.remember_current_view();
        } else {
            run_command(state, hwnd, id);
        }
    }
}

/// Open the settings menu, from the keyboard or from the palette.
///
/// Anchored under the gear button when the bar is showing, and under the top
/// of the focused pane when it is not \u2014 a menu has to come from
/// somewhere, and it is still the same menu.
pub fn do_settings(state: &mut AppState, hwnd: HWND) {
    let layout = state.layout();
    let at = BAR
        .iter()
        .position(|b| b.action == BarAction::Menu(BarMenu::Settings))
        .and_then(|i| layout.bar_items.get(i).copied())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| {
            // No bar to hang from: the top-right of the pane, which is where
            // the button would have been.
            let b = layout.pane(state.focused).bounds;
            crate::layout::Rect::new(b.right(), b.y, 0, 0)
        });
    bar_menu(state, hwnd, BarMenu::Settings, at);
}

/// Map or disconnect a network drive.
///
/// Windows owns both dialogs \u2014 they know about credentials, saved
/// connections and the "reconnect at sign-in" box, none of which this app has
/// any business reimplementing.
pub fn do_network_drive(state: &mut AppState, hwnd: HWND, connect: bool) {
    use windows::Win32::NetworkManagement::WNet::{
        WNetConnectionDialog, WNetDisconnectDialog, RESOURCETYPE_DISK,
    };
    if state.modal {
        return;
    }
    state.modal = true;
    let result = unsafe {
        if connect {
            WNetConnectionDialog(hwnd, RESOURCETYPE_DISK.0)
        } else {
            WNetDisconnectDialog(Some(hwnd), RESOURCETYPE_DISK.0)
        }
    };
    state.modal = false;

    // 0 is success and 0xFFFFFFFF is "the user cancelled", which is not a
    // failure and not worth a message box.
    let result = result.0;
    if result != 0 && result != u32::MAX {
        report_error(
            hwnd,
            if connect { "Map network drive" } else { "Disconnect" },
            &format!("Windows reported error {}", result),
        );
    }
    // Either way the drive list may have changed.
    state.drives = fs::drives();
}

/// Size the folders in a pane's listing, if the setting is on.
///
/// ponytail: the whole listing in one pass rather than only the rows on
/// screen, capped at `AUTO_SIZE_LIMIT` folders. On-screen-only sounds cheaper
/// but is not \u2014 twenty visible folders holding a hundred thousand files
/// each cost the same walk \u2014 and it needs sizing to re-trigger on every
/// scroll. Per-row sizing driven by the scroll position is the upgrade.
pub const AUTO_SIZE_LIMIT: usize = 256;

pub fn auto_folder_sizes(state: &mut AppState, hwnd: HWND, pid: PaneId) {
    if !state.folder_sizes {
        return;
    }
    let pane = state.pane(pid);
    let dir = pane.current_path().to_string();
    if dir.is_empty() || crate::shellns::is_shell_path(&dir) {
        return;
    }
    let names: Vec<String> = pane
        .list()
        .entries
        .iter()
        .filter(|e| e.is_dir && !e.is_reparse)
        .map(|e| e.name.clone())
        .collect();
    if names.is_empty() {
        return;
    }
    if names.len() > AUTO_SIZE_LIMIT {
        state.status_override = Some(format!(
            "{} folders here, so sizing them is still on request",
            names.len()
        ));
        return;
    }
    let tab_id = state.pane(pid).active_tab_id();
    spawn_dir_sizes(hwnd, pid, tab_id, dir, names);
}

/// Rebuild everything that is measured in text: the formats the renderer
/// draws with, and the metrics the layout measures with.
///
/// One function because the two cannot disagree — a row sized for 100% text
/// with 150% text in it is a clipped row.
fn apply_font(state: &mut AppState, hwnd: HWND) {
    let (font, size) = (state.font.clone(), state.font_size);
    if let Err(e) = state.renderer.set_font(&font, size) {
        report_error(hwnd, "Font", &format!("{}", e));
        return;
    }
    state.apply_metrics();
    invalidate(hwnd);
}

/// Every font installed, filtered as you type.
pub fn do_font(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let here = state.font.clone();
    let families = state.renderer.font_families();
    if families.is_empty() {
        state.status_override = Some("Could not read the installed fonts".into());
        return;
    }
    let items: Vec<palette::Item> = families
        .iter()
        .map(|f| palette::Item {
            label: f.clone(),
            detail: if f.eq_ignore_ascii_case(&here) {
                "current".into()
            } else {
                String::new()
            },
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Font", items);
    state.modal = false;

    if let Some(i) = chosen {
        state.font = families[i].clone();
        apply_font(state, hwnd);
    }
}

/// Text size, as a percentage of the design size.
pub fn do_font_size(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let here = state.font_size;
    let items: Vec<palette::Item> = crate::layout::FONT_STEPS
        .iter()
        .map(|pct| palette::Item {
            label: format!("{}%", pct),
            detail: match (*pct == here, *pct == 100) {
                (true, _) => "current".into(),
                (_, true) => "default".into(),
                _ => String::new(),
            },
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Text size", items);
    state.modal = false;

    if let Some(i) = chosen {
        state.font_size = crate::layout::FONT_STEPS[i];
        apply_font(state, hwnd);
    }
}

/// How much room a row gets beyond what its text needs.
pub fn do_density(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let here = state.density;
    let items: Vec<palette::Item> = crate::layout::DENSITY_STEPS
        .iter()
        .map(|(name, pct)| palette::Item {
            label: (*name).to_string(),
            detail: if *pct == here {
                "current".into()
            } else {
                format!("{}%", pct)
            },
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Row density", items);
    state.modal = false;

    if let Some(i) = chosen {
        state.density = crate::layout::DENSITY_STEPS[i].1;
        apply_font(state, hwnd);
    }
}

/// Pick a theme by name. Ctrl+Shift+D stays the light switch; this is the
/// rest of the table.
pub fn do_theme(state: &mut AppState, hwnd: HWND) {
    use crate::theme::{Theme, THEMES};
    if state.modal {
        return;
    }
    let here = state.theme;
    let items: Vec<palette::Item> = THEMES
        .iter()
        .enumerate()
        .map(|(i, t)| palette::Item {
            label: t.name.to_string(),
            detail: match (i == here.index(), t.dark) {
                (true, _) => "current".into(),
                (_, true) => "dark".into(),
                (_, false) => "light".into(),
            },
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Theme", items);
    state.modal = false;

    if let Some(i) = chosen {
        crate::input::set_theme(state, hwnd, Theme::at(i));
    }
}

/// Remember the pane tree under a name. Saving over a name replaces it: two
/// layouts called the same thing is the shape of a bug, not a feature.
pub fn do_save_layout(state: &mut AppState, hwnd: HWND) {
    let suggested = format!("Layout {}", state.saved_layouts.len() + 1);
    let Some(name) = crate::prompt::prompt_text(hwnd, state, "Save layout", "Name:", &suggested)
    else {
        return;
    };
    // "=" is the settings file's own separator and the name is written before
    // the tree, so a name carrying one would read back as a shorter name and
    // a tree nobody can parse.
    let name = name.trim().replace('=', "-");
    if name.is_empty() {
        return;
    }
    let tree = crate::config::tree_to_text(&state.layout_tree);
    match state
        .saved_layouts
        .iter_mut()
        .find(|(n, _)| n.eq_ignore_ascii_case(&name))
    {
        Some(slot) => slot.1 = tree,
        None => state.saved_layouts.push((name.clone(), tree)),
    }
    state.status_override = Some(format!("Layout saved as {}", name));
}

/// Pick one of them and show it.
pub fn do_layouts(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    if state.saved_layouts.is_empty() {
        state.status_override = Some("No layouts saved yet".into());
        return;
    }
    let here = crate::config::tree_to_text(&state.layout_tree);
    let items: Vec<palette::Item> = state
        .saved_layouts
        .iter()
        .map(|(name, tree)| palette::Item {
            label: name.clone(),
            detail: if *tree == here {
                "current".into()
            } else {
                let panes = crate::config::tree_checked(tree).map(|n| n.count()).unwrap_or(0);
                format!("{} panes", panes)
            },
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Layouts", items);
    state.modal = false;

    let Some(tree) = chosen
        .and_then(|i| state.saved_layouts.get(i))
        .and_then(|(_, tree)| crate::config::tree_checked(tree))
    else {
        return;
    };
    state.set_layout(tree);
}

/// The menu inside a multi-row rename.
///
/// Editing commands, then the three things that differ per row \u2014 a number,
/// a date, the file's own date. They go in expanded rather than as a token, so
/// each row shows what it will be called.
pub fn multi_rename_menu(state: &mut AppState, hwnd: HWND, at: POINT) {
    const COPY: usize = RENAME_FIRST;
    const PASTE: usize = RENAME_FIRST + 1;
    const ALL: usize = RENAME_FIRST + 2;
    const STEM: usize = RENAME_FIRST + 3;
    const NUMBER: usize = RENAME_FIRST + 4;
    const TODAY: usize = RENAME_FIRST + 5;
    const FILE_DATE: usize = RENAME_FIRST + 6;

    if state.multi_rename.is_none() {
        return;
    }
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut themed = crate::menu::ThemedMenu::new(state.theme, state.dpi);
        themed.add(menu, COPY, "Copy", "Ctrl+C");
        themed.add(menu, PASTE, "Paste", "Ctrl+V");
        themed.add(menu, ALL, "Select all", "Ctrl+A");
        themed.add(menu, STEM, "Select name without extension", "");
        themed.separator(menu);
        themed.add(menu, NUMBER, "Insert number", "");
        themed.add(menu, TODAY, "Insert today's date", "");
        themed.add(menu, FILE_DATE, "Insert file date", "");
        themed.apply(menu);

        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        let pasted = (cmd.0 as usize == PASTE)
            .then(|| ops::clipboard_read_text(hwnd))
            .flatten();
        let Some(m) = state.multi_rename.as_mut() else {
            return;
        };
        match cmd.0 as usize {
            COPY => {
                let text = m.selected_text();
                if !text.is_empty() {
                    let _ = ops::clipboard_write_text(hwnd, &text);
                }
            }
            PASTE => {
                if let Some(text) = pasted {
                    // One line only: a name cannot contain a newline, and a
                    // multi-line paste is almost always an accident.
                    let line = text.lines().next().unwrap_or_default().to_string();
                    m.insert(&line);
                }
            }
            ALL => m.select_all(),
            STEM => m.select_stem(),
            NUMBER => m.insert_token("{##}"),
            TODAY => {
                let today = today_text();
                m.insert(&today);
            }
            FILE_DATE => m.insert_token("{d}"),
            _ => {}
        }
        invalidate(hwnd);
    }
}

/// Today, as `YYYY-MM-DD`, from the same formatter the listing's dates use.
fn today_text() -> String {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    // The listing keeps its times as one number, which is what the formatter
    // takes; a FILETIME is that number in two halves.
    let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    fs::format_filetime(ticks)
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Take over opening folders, or hand it back.
///
/// One command for both directions: which one it is depends on what the
/// registry currently says, and the message box spells out what will change
/// before anything is written. This is a system default, so it is never
/// silent and never a side effect of anything else.
pub fn do_default_app(state: &mut AppState, hwnd: HWND) {
    use crate::default_app;

    let exe = match default_app::exe_path() {
        Ok(e) => e,
        Err(e) => {
            report_error(hwnd, "Default file manager", &e);
            return;
        }
    };

    if default_app::is_default() {
        let ok = crate::app::confirm(
            hwnd,
            "Restore Windows Explorer",
            "Folders, drives and This PC will open in Windows Explorer again.\n\n\
             Whatever these keys said before File Xplorer claimed them is put \
             back exactly. Nothing outside your own user account is touched.\n\n\
             Restore Windows Explorer?",
        );
        if !ok {
            return;
        }
        match default_app::restore() {
            Ok(()) => state.status_override = Some("Windows Explorer restored".into()),
            Err(e) => report_error(hwnd, "Restore Windows Explorer", &e),
        }
        return;
    }

    let ok = crate::app::confirm(
        hwnd,
        "Default file manager",
        &format!(
            "Folders, drives and This PC will open in File Xplorer instead of \
             Windows Explorer.\n\n\
             This writes three keys under HKEY_CURRENT_USER only \u{2014} your \
             account, not the machine \u{2014} and records what each one said \
             first so this command can put them back. Windows Explorer itself \
             keeps working.\n\n{}\n\n\
             Make File Xplorer the default?",
            default_app::command_for(&exe)
        ),
    );
    if !ok {
        return;
    }
    match default_app::make_default() {
        Ok(()) => {
            state.status_override = Some("File Xplorer opens folders now".into());
        }
        Err(e) => report_error(hwnd, "Default file manager", &e),
    }
}

/// Run the shell verb whose menu entry reads `name`, if it offered one.
///
/// ponytail: matched on the menu's text, so a Windows running in another
/// language will not find it and says so. The upgrade is
/// IDataTransferManagerInterop, which is the documented way to raise the share
/// sheet and a great deal more code than this.
fn invoke_named(state: &mut AppState, hwnd: HWND, name: &str) -> bool {
    let pid = state.focused;
    let selected = state.pane(pid).selected_pidls();
    if selected.is_empty() {
        return false;
    }
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return false };
        let shell = shellmenu::append(menu, hwnd, &selected);
        let found = shellmenu::list(menu)
            .into_iter()
            .find(|a| a.label.eq_ignore_ascii_case(name));
        let _ = DestroyMenu(menu);
        match (found, &shell) {
            (Some(a), Some(shell)) => {
                shell.invoke(hwnd, a.id);
                true
            }
            _ => false,
        }
    }
}

/// Every verb the shell offers for the selection, filtered as you type.
///
/// The menu is built and never shown. `QueryContextMenu` fills an HMENU
/// whether or not anyone looks at it, and a searchable list is a better way to
/// read forty entries than a column that runs off the bottom of the screen.
///
/// `pin` picks from the same list but stores the choice instead of running it,
/// which is how a verb gets hoisted to the top of the context menu.
pub fn do_actions(state: &mut AppState, hwnd: HWND, pin: bool) {
    if state.modal {
        return;
    }
    let pid = state.focused;
    let selected = state.pane(pid).selected_pidls();
    if selected.is_empty() {
        state.status_override = Some("Select a file or folder first".into());
        return;
    }

    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        // Held until after the choice: the id is an offset into this menu's
        // command range, so the object that allocated it has to invoke it.
        let shell = shellmenu::append(menu, hwnd, &selected);
        let actions = shellmenu::list(menu);
        // The menu stays until the picker is gone: the little bitmaps beside
        // the rows belong to it, and destroying it takes them away.
        if actions.is_empty() {
            let _ = DestroyMenu(menu);
            state.status_override = Some("The shell offered nothing here".into());
            return;
        }
        let items: Vec<palette::Item> = actions
            .iter()
            .map(|a| palette::Item {
                label: a.label.clone(),
                detail: if state
                    .pin_actions
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(&a.label))
                {
                    "pinned".into()
                } else {
                    String::new()
                },
                icon: None,
                bitmap: a.bitmap,
                })
            .collect();

        state.modal = true;
        // Running one opens where the pointer is, the way the menu it came
        // from did. Pinning is a different question about the same list, and
        // a question needs the title only the centred box has room for.
        let chosen = if pin {
            palette::pick(hwnd, state.dpi, state.theme, "Pin an action", items)
        } else {
            let mut at = POINT::default();
            let _ = GetCursorPos(&mut at);
            palette::pick_at(hwnd, state.dpi, state.theme, items, (at.x, at.y))
        };
        state.modal = false;
        let _ = DestroyMenu(menu);

        let Some(i) = chosen else { return };
        let action = &actions[i];
        if pin {
            let now = state.toggle_action_pin(&action.label);
            state.status_override = Some(format!(
                "{} {}",
                if now { "Pinned" } else { "Unpinned" },
                action.label
            ));
            return;
        }
        if let Some(shell) = &shell {
            shell.invoke(hwnd, action.id);
            // The shell just changed something; find out what.
            let req = state.pane_mut(pid).refresh();
            start_load(hwnd, pid, req);
        }
    }
}

/// Show every command, filtered as you type.
pub fn show_palette(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    // The shortcut shown is whatever is bound now, not the default baked into
    // the table: a rebound command that still advertised its old chord would
    // be worse than showing none.
    let items: Vec<(usize, palette::Item)> = COMMANDS
        .iter()
        .filter(|c| c.id != CMD_PALETTE)
        .map(|c| {
            (
                c.id,
                palette::Item {
                    label: c.label.to_string(),
                    detail: state.bindings.text_for(c.id),
                    icon: None,
                    bitmap: None,
                        },
            )
        })
        .collect();
    let ids: Vec<usize> = items.iter().map(|(id, _)| *id).collect();
    let items: Vec<palette::Item> = items.into_iter().map(|(_, it)| it).collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Commands", items);
    state.modal = false;

    if let Some(i) = chosen {
        if let Some(id) = ids.get(i).copied() {
            run_command(state, hwnd, id);
        }
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

/// Walk each named folder on a worker thread and post the totals back as they
/// arrive.
///
/// In batches rather than one message per folder: the list rebuilds itself on
/// every batch, and rebuilding once per folder would be a filter and a sort per
/// answer. In batches rather than one message at the end, because one slow
/// folder should not hold back the twenty that were quick — which is the
/// difference between automatic sizing being usable and being a spinner.
pub fn spawn_dir_sizes(hwnd: HWND, pid: PaneId, tab_id: u64, dir: String, names: Vec<String>) {
    const BATCH: std::time::Duration = std::time::Duration::from_millis(400);
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let post = |sizes: Vec<(String, u64)>| -> bool {
            if sizes.is_empty() {
                return true;
            }
            let raw = Box::into_raw(Box::new(SizesDone { pid, tab_id, sizes }));
            let posted = unsafe {
                PostMessageW(
                    Some(owner.hwnd()),
                    WM_APP_SIZES_DONE,
                    WPARAM(0),
                    LPARAM(raw as isize),
                )
            };
            if posted.is_err() {
                // Nobody is going to take ownership of it now.
                unsafe { drop(Box::from_raw(raw)) };
                return false;
            }
            true
        };

        let mut batch: Vec<(String, u64)> = Vec::new();
        let mut since = std::time::Instant::now();
        for n in names {
            let size = fs::dir_size(&fs::path_join(&dir, &n));
            batch.push((n, size));
            if since.elapsed() >= BATCH {
                if !post(std::mem::take(&mut batch)) {
                    return; // the window has gone
                }
                since = std::time::Instant::now();
            }
        }
        post(batch);
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

/// Finish a multi-row rename. `commit` applies every changed name as one
/// operation, so one undo puts all of them back.
///
/// A name that cannot exist stops the whole batch and leaves the editor open:
/// half a rename applied is worse than none of it, and the row that is wrong
/// is still on screen to be fixed.
pub fn finish_multi_rename(state: &mut AppState, hwnd: HWND, commit: bool) {
    let Some(editor) = state.multi_rename.take() else {
        return;
    };
    invalidate(hwnd);
    if !commit {
        return;
    }
    let dir = state.pane(editor.pid).current_path().to_string();
    match editor.plan() {
        Ok(plan) if plan.is_empty() => {}
        Ok(plan) => {
            let items = plan
                .into_iter()
                .map(|(old, new)| (fs::path_join(&dir, &old), new))
                .collect();
            state.status_override = Some("Renaming\u{2026}".into());
            spawn_op(hwnd, ops::Op::RenameMany { items });
        }
        Err(why) => {
            report_error(hwnd, "Rename", &why);
            state.multi_rename = Some(editor);
        }
    }
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
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Change a shortcut", items);
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

/// The go-to list, in the order it is offered: visited, pinned, named,
/// mounted, open. A folder reached two ways is one entry — the first way wins,
/// which is why recents lead: the label you last saw is the label you get.
fn go_to_list(
    recent: &[String],
    pins: &[String],
    places: &[crate::app::Shortcut],
    drives: &[fs::Drive],
    tree: &[crate::tree::TreeRow],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add = |path: &str, label: String| {
        if !path.is_empty() && !label.is_empty() && seen.insert(path.to_lowercase()) {
            out.push((label, path.to_string()));
        }
    };

    for p in recent {
        add(p, fs::path_leaf(p));
    }
    for p in pins {
        add(p, fs::path_leaf(p));
    }
    // The sidebar's own shortcuts — Downloads, Documents, Home. Named there,
    // so named here too rather than reduced to the folder's leaf.
    for sc in places {
        add(&sc.path, sc.label.clone());
    }
    for d in drives {
        // The letter goes in the label, not just the detail: "c:" is how
        // people reach for a drive, and only the label is matched against.
        add(
            &d.root,
            format!("{} {}", d.root.trim_end_matches('\\'), d.label),
        );
    }
    for row in tree {
        add(&row.path, row.label.clone());
    }
    out
}

/// Everywhere this window already knows about, as one fuzzy list: folders
/// visited, folders pinned, the sidebar's places, the drives, and whatever the
/// tree has open.
///
/// Ctrl+L stays a path box with the shell's own completion, because that is
/// the right tool for a path you can type. This is for the ones you would
/// rather not type.
pub fn do_places(state: &mut AppState, hwnd: HWND) {
    if state.modal {
        return;
    }
    let roots: Vec<String> = state.drives.iter().map(|d| d.root.clone()).collect();
    let found = go_to_list(
        &state.recent,
        &state.pins,
        &state.places,
        &state.drives,
        &state.tree.rows(&roots),
    );
    if found.is_empty() {
        state.status_override = Some("No folders to go to yet".into());
        return;
    }

    let paths: Vec<String> = found.iter().map(|(_, p)| p.clone()).collect();
    let items: Vec<palette::Item> = found
        .into_iter()
        .map(|(label, path)| palette::Item {
            label,
            detail: path,
            icon: None,
            bitmap: None,
        })
        .collect();

    state.modal = true;
    let chosen = palette::pick(hwnd, state.dpi, state.theme, "Go to", items);
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
            dir = fs::child_path(&dir, e);
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
    if state.pane_count() < 2 {
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
    spawn_transfer(state, hwnd, op);
}

pub fn copy_or_move_to_other(state: &mut AppState, hwnd: HWND, move_it: bool) {
    if state.pane_count() < 2 {
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
    spawn_transfer(state, hwnd, op);
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
    // More than one selected: type over all of them at once, in place.
    if !state.modal && state.pane(pid).list().selection_count() > 1 {
        let mut rows: Vec<u32> = state
            .pane(pid)
            .list()
            .selected_set()
            .iter()
            .copied()
            .collect();
        rows.sort_unstable();
        let entries = state.pane(pid).list().entries.clone();
        state.multi_rename = crate::multi_rename::MultiRename::begin(rows, pid, &entries);
        if state.multi_rename.is_some() {
            invalidate(hwnd);
            return;
        }
    }
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

/// A little in from the row's left edge, so the menu does not sit flush
/// against the pane.
fn m_pad(layout: &crate::layout::Layout) -> i32 {
    layout.metrics.pad
}

pub fn show_context_menu(state: &mut AppState, hwnd: HWND, screen_x: i32, screen_y: i32) {
    let mut pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    // A rename in progress owns the right-click: the file commands would act
    // on names that are being typed and do not exist yet.
    if state.multi_rename.is_some() {
        if screen_x == -1 && screen_y == -1 {
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
        }
        multi_rename_menu(state, hwnd, pt);
        return;
    }
    // Shift+F10 sends (-1, -1). Anchor to the cursor row — which is what the
    // keyboard is pointing at — rather than to wherever the mouse happens to
    // be sitting, which may be over another pane, the sidebar, or nothing.
    // Anchoring to the mouse is why the keyboard menu used to offer the
    // folder's commands while a file was selected.
    if screen_x == -1 && screen_y == -1 {
        let pid = state.focused;
        let layout = state.layout();
        let list = state.pane(pid).list();
        let anchor = list
            .cursor()
            .and_then(|i| layout.pane(pid).cell(i, list.scroll_px()))
            .unwrap_or(layout.pane(pid).list);
        // The row's bottom edge *minus one*, so the menu drops from under the
        // row while the point still lands inside it: the hit test that decides
        // whether this is the file's menu or the folder's runs on this point,
        // and one pixel lower is already the next row.
        pt = POINT {
            x: anchor.x + m_pad(&layout),
            y: anchor.y + anchor.h - 1,
        };
        unsafe {
            let _ = ClientToScreen(hwnd, &mut pt);
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
    // Nothing writes inside an archive or in the shell namespace, so none of
    // the commands that `spawn_op_tagged` would refuse are offered. Offering
    // them and then reporting an error is a worse way to say the same thing.
    let here = state.pane(pid).current_path().to_string();
    let in_archive = crate::archive::split(&here).is_some();
    let writable = !in_archive && !crate::shellns::is_shell_path(&here);
    let pinned = state.is_pinned(state.pane(pid).current_path());

    // Our own commands, then everything installed software registered, with
    // the shell's own submenus left as submenus. A verb buried three menus
    // deep is still hard to point at, which is what *Search actions...* is
    // for: the flat, typeable list of the same menu.
    let binds = state.bindings.clone();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut themed = crate::menu::ThemedMenu::new(state.theme, state.dpi);
        // A macro rather than a closure: a closure would hold `themed`
        // borrowed for the whole block, and the separators need it too. The
        // shortcut shown is whatever is bound now, rather than a second copy
        // of the binding free to drift from the one that fires.
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
            add!(state.pane_count() > 1, CMD_COPY_TO_OTHER, "Copy to next pane");
            add!(
                state.pane_count() > 1 && writable,
                CMD_MOVE_TO_OTHER,
                "Move to next pane"
            );
            add!(in_archive, CMD_EXTRACT, "Extract here");
            add!(writable && !in_archive, CMD_ARCHIVE, "Add to archive\u{2026}");
            themed.separator(menu);
            add!(writable && count == 1, CMD_RENAME, "Rename");
            add!(writable && count > 1, CMD_BATCH_RENAME, "Rename with a pattern");
            add!(writable, CMD_DELETE, "Delete");
            themed.separator(menu);
            add!(count > 0, CMD_SHARE, "Share");
            add!(true, CMD_ACTIONS, "Search actions\u{2026}");
            add!(count == 1, CMD_PROPERTIES, "Properties");
        } else {
            // What this folder can do. Everything else lives in the palette,
            // which is one line further down and searchable.
            add!(writable, CMD_NEW_FOLDER, "New folder");
            add!(writable && ops::clipboard_has_files(), CMD_PASTE, "Paste");
            add!(in_archive, CMD_EXTRACT, "Extract all");
            themed.separator(menu);
            add!(true, CMD_GOTO, "Go to path\u{2026}");
            add!(true, CMD_TERMINAL, "Open terminal here");
            add!(
                true,
                CMD_PIN,
                if pinned { "Unpin this folder" } else { "Pin this folder" }
            );
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
        let selected = if row.is_some() {
            state.pane(pid).selected_pidls()
        } else {
            Vec::new()
        };
        let shell = shellmenu::append(menu, hwnd, &selected);
        // The shell's half arrives in the system's colours. Take it over, so
        // the menu is one menu rather than two halves of different ones.
        crate::menu::adopt(menu);

        // Pinned verbs go above our own items: reading down to find them is
        // the problem pinning exists to solve. They keep the shell's id, so
        // the dispatch below cannot tell the two copies apart, which is the
        // point.
        if shell.is_some() && !state.pin_actions.is_empty() {
            let offered = shellmenu::list(menu);
            let mut at = 0;
            for name in &state.pin_actions {
                if let Some(a) = offered.iter().find(|a| a.label.eq_ignore_ascii_case(name)) {
                    themed.insert(menu, at, a.id as usize, &a.label);
                    at += 1;
                }
            }
            if at > 0 {
                themed.insert_separator(menu, at);
            }
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
        if id == 0 {
            return; // dismissed
        }
        if id >= shellmenu::SHELL_ID_FIRST {
            if let Some(shell) = &shell {
                shell.invoke(hwnd, id);
            }
            // The shell just changed something; find out what.
            let req = state.pane_mut(pid).refresh();
            start_load(hwnd, pid, req);
        } else {
            run_command(state, hwnd, id as usize);
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
                    let path = fs::child_path(pane.current_path(), entry);
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
        CMD_ACTIONS => do_actions(state, hwnd, false),
        CMD_DEFAULT_APP => do_default_app(state, hwnd),
        CMD_SHARE => {
            if !invoke_named(state, hwnd, "Share") {
                state.status_override =
                    Some("Nothing to share, or Windows offered no Share here".into());
            }
        }
        CMD_PIN_ACTION => do_actions(state, hwnd, true),
        CMD_GOTO => do_goto(state, hwnd),
        CMD_RECENT => do_places(state, hwnd),
        CMD_TOGGLE_BAR => state.command_bar = !state.command_bar,
        CMD_SPLIT_RIGHT | CMD_SPLIT_DOWN => {
            let here = state.pane(pid).current_path().to_string();
            match state.split_focused(cmd == CMD_SPLIT_RIGHT) {
                Some(new) => {
                    // The new pane opens where you were: two views of one
                    // folder, then navigate one of them away. Starting it
                    // empty would make splitting a two-step gesture.
                    let req = state.pane_mut(new).navigate(&here);
                    spawn_dir_load(hwnd, new, req);
                    state.rewatch(new, hwnd);
                }
                None => {
                    state.status_override =
                        Some(format!("Already showing {} panes", crate::layout::MAX_PANES))
                }
            }
        }
        CMD_CLOSE_PANE => {
            if !state.close_focused() {
                state.status_override = Some("This is the only pane".into());
            }
        }
        CMD_NEW_WINDOW => {
            // Opens on this folder rather than restoring the session: the
            // session belongs to one window, and this is not it.
            let here = state.pane(pid).current_path().to_string();
            if let Err(e) = crate::create_window(crate::WindowOpts {
                start: (!here.is_empty()).then_some(here),
                owns_session: false,
            }) {
                report_error(hwnd, "New window", &ops::format_hresult(&e));
            }
        }
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
        CMD_THEME => do_theme(state, hwnd),
        CMD_SAVE_LAYOUT => do_save_layout(state, hwnd),
        CMD_LAYOUTS => do_layouts(state, hwnd),
        CMD_FONT => do_font(state, hwnd),
        CMD_FONT_SIZE => do_font_size(state, hwnd),
        CMD_DENSITY => do_density(state, hwnd),
        CMD_SETTINGS => do_settings(state, hwnd),
        CMD_MAP_DRIVE | CMD_DISCONNECT_DRIVE => do_network_drive(state, hwnd, cmd == CMD_MAP_DRIVE),
        CMD_FOLDER_SIZES => {
            state.folder_sizes = !state.folder_sizes;
            state.status_override = Some(
                if state.folder_sizes {
                    "Folder sizes will be calculated on opening a folder"
                } else {
                    "Folder sizes are calculated on request"
                }
                .into(),
            );
            if state.folder_sizes {
                auto_folder_sizes(state, hwnd, state.focused);
            }
        }
        CMD_TOGGLE_HIDDEN => toggle_hidden(state, hwnd),
        CMD_TOGGLE_SIDEBAR => {
            state.sidebar_visible = !state.sidebar_visible;
        }
        CMD_GRID => {
            // Toggling goes to the default size and back, rather than to
            // whatever size was last used: a toggle that lands somewhere
            // different each time is not a toggle.
            state.icons = if state.icons == crate::layout::ICONS_OFF {
                crate::layout::DEFAULT_ICONS
            } else {
                crate::layout::ICONS_OFF
            };
            // Choosing a view by hand is a statement about this folder, the
            // same as choosing a sort.
            state.remember_current_view();
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
            let i = state.pane(pid).active_tab_index;
            crate::input::close_tab(state, hwnd, pid, i);
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
    fn a_plain_ctrl_letter_chord_reaches_its_command() {
        // Ctrl+N was the first command with no hardcoded fallback in the key
        // handler, so it was also the first that would have failed silently if
        // the binding table were not really being consulted.
        let b = crate::keys::Bindings::from_defaults(&default_bindings());
        let ctrl_n = crate::keys::Chord::new(b'N' as u16, true, false, false);
        assert_eq!(b.command_for(ctrl_n), Some(CMD_NEW_WINDOW));

        let ctrl_shift_p = crate::keys::Chord::new(b'P' as u16, true, true, false);
        assert_eq!(b.command_for(ctrl_shift_p), Some(CMD_PALETTE));
    }

    #[test]
    fn command_ids_and_labels_resolve_both_ways() {
        for c in COMMANDS {
            assert_eq!(id_for_label(c.label), Some(c.id), "{}", c.label);
            assert_eq!(label_for(c.id), c.label);
        }
    }

    #[test]
    fn the_menus_id_blocks_cannot_swallow_one_another() {
        // Every block is FIRST + index into a table, and `bar_menu` claims
        // them in descending order. A table that outgrew the gap below the
        // next block would be read as the wrong setting entirely \u2014 a
        // theme applied as a density \u2014 so the gaps are checked against
        // the tables rather than eyeballed.
        assert!(COMMANDS.iter().all(|c| c.id < SORT_FIRST), "commands sit below");
        assert!(SORT_FIRST + 4 <= ORDER_ASC);
        assert!(ORDER_ASC < ORDER_DESC && ORDER_DESC < ICON_FIRST);
        assert!(ICON_FIRST + 1 + crate::layout::ICON_STEPS.len() <= ICON_LIST);
        assert!(ICON_LIST < ICON_TILES && ICON_TILES < BAR_FIRST);
        assert!(BAR_FIRST + BAR.len() <= THEME_FIRST);
        assert!(THEME_FIRST + crate::theme::THEMES.len() <= TEXT_FIRST);
        assert!(TEXT_FIRST + crate::layout::FONT_STEPS.len() <= DENSITY_FIRST);
        assert!(DENSITY_FIRST + crate::layout::DENSITY_STEPS.len() <= RENAME_FIRST);
        assert!(!crate::layout::DENSITY_STEPS.is_empty());
    }

    #[test]
    fn every_bar_button_can_say_what_it_is() {
        // The overflow menu shows names, not glyphs, so a button whose name is
        // empty would appear there as a blank row nobody can read.
        for it in BAR {
            assert!(!bar_name(it).is_empty(), "{:?} has no name", it.glyph);
        }
    }

    #[test]
    fn the_go_to_list_offers_each_folder_once_under_its_first_name() {
        let drive = fs::Drive {
            root: "C:\\".to_string(),
            label: "Local Disk".into(),
            total_bytes: 0,
            free_bytes: 0,
        };
        let place = crate::app::Shortcut {
            label: "Downloads".into(),
            path: "C:\\Users\\me\\Downloads".into(),
        };
        let row = crate::tree::TreeRow {
            path: "C:\\Users\\me\\downloads".into(), // the same folder, spelled otherwise
            label: "downloads".into(),
            depth: 1,
            expanded: false,
        };
        let recent = vec!["C:\\Users\\me\\Downloads".to_string()];

        let all = go_to_list(&recent, &[], &[place], &[drive], &[row]);
        let labels: Vec<&str> = all.iter().map(|(l, _)| l.as_str()).collect();
        // Once, under the name the recents gave it, and case is not a second
        // folder. The drive still gets its letter in the label, which is how
        // typing "c:" reaches it.
        assert_eq!(labels, vec!["Downloads", "C: Local Disk"]);
    }

    #[test]
    fn a_drive_already_visited_keeps_the_name_it_was_visited_under() {
        let drive = fs::Drive {
            root: "C:\\".to_string(),
            label: "Local Disk".into(),
            total_bytes: 0,
            free_bytes: 0,
        };
        let all = go_to_list(&["C:\\".to_string()], &[], &[], &[drive], &[]);
        // One entry, not two. A root's leaf is the root itself, so it reads
        // the same either way — and "c:" still matches it.
        assert_eq!(all, vec![("C:\\".to_string(), "C:\\".to_string())]);
    }
}

