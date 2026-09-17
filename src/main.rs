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

mod config;
mod file_list;
mod fs;
mod icons;
mod layout;
mod ops;
mod pane;
mod renderer;
mod theme;
mod watch;

use std::collections::HashSet;
use std::time::Instant;

use windows::core::{Result, BOOL, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use config::{Config, UNSET};
use file_list::SelectMode;
use layout::{Hit, Layout, Metrics, NavButton, PaneInput, Rect, SidebarEntry, Side};
use pane::{LoadRequest, Pane};
use renderer::{PaneView, Renderer, SidebarView};
use theme::Theme;
use watch::Watcher;

/// A finished directory read, posted back from a worker thread.
const WM_APP_DIR_LOADED: u32 = WM_APP + 1;
/// A finished file operation, posted back from a worker thread.
const WM_APP_OP_DONE: u32 = WM_APP + 2;
/// Calculated folder sizes, posted back from a worker thread.
const WM_APP_SIZES_DONE: u32 = WM_APP + 3;
/// A watched directory changed on disk. wParam is the pane index.
const WM_APP_DIR_CHANGED: u32 = WM_APP + 4;

/// Timer ids for coalescing change notifications, one per pane.
const TIMER_WATCH_LEFT: usize = 1;
const TIMER_WATCH_RIGHT: usize = 2;
/// A burst of writes (an unzip, a build) should cost one reload, not hundreds.
const WATCH_DEBOUNCE_MS: u32 = 250;

const FALLBACK_PATH: &str = "C:\\";
/// How long a type-to-select prefix stays alive between keystrokes.
const TYPE_AHEAD_RESET_MS: u128 = 900;

/// WM_MOUSELEAVE is not re-exported by the windows crate. It must be a real
/// `const` and not a plain binding, or the match arm below becomes an
/// irrefutable pattern that swallows every message after it.
const WM_MOUSELEAVE: u32 = 0x02A3;

// Context menu command ids.
const CMD_OPEN: usize = 100;
const CMD_OPEN_NEW_TAB: usize = 101;
const CMD_CUT: usize = 102;
const CMD_COPY: usize = 103;
const CMD_PASTE: usize = 104;
const CMD_DELETE: usize = 105;
const CMD_RENAME: usize = 106;
const CMD_NEW_FOLDER: usize = 107;
const CMD_PROPERTIES: usize = 108;
const CMD_COPY_TO_OTHER: usize = 109;
const CMD_MOVE_TO_OTHER: usize = 110;
const CMD_REFRESH: usize = 111;
const CMD_CALC_SIZES: usize = 112;
const CMD_SYNC_SCROLL: usize = 113;
const CMD_COMPARE: usize = 114;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn loword(v: u32) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}
fn hiword(v: u32) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// Client coordinates from an LPARAM, sign-extended.
///
/// The old code masked with 0xFFFF and stored the result in a u32, so a drag
/// that left the window on the left or top produced coordinates near 65535.
fn mouse_pos(lparam: LPARAM) -> (i32, i32) {
    let v = lparam.0 as u32;
    (loword(v), hiword(v))
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}
fn ctrl_down() -> bool {
    key_down(VK_CONTROL)
}
fn shift_down() -> bool {
    key_down(VK_SHIFT)
}
fn alt_down() -> bool {
    key_down(VK_MENU)
}

fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

fn side_from_index(i: usize) -> Side {
    if i == 0 {
        Side::Left
    } else {
        Side::Right
    }
}

fn select_mode() -> SelectMode {
    if shift_down() {
        SelectMode::Range
    } else if ctrl_down() {
        SelectMode::Toggle
    } else {
        SelectMode::Replace
    }
}

struct SizesDone {
    side: Side,
    tab_id: u64,
    sizes: Vec<(String, u64)>,
}

struct DirLoaded {
    side: Side,
    req: LoadRequest,
    result: std::result::Result<Vec<fs::FileEntry>, String>,
}

/// A named location in the sidebar's Places section.
struct Shortcut {
    label: String,
    path: String,
}

/// Which sidebar section a header row belongs to.
const SECTION_DRIVES: usize = 0;
const SECTION_PLACES: usize = 1;

/// What clicking a sidebar row should do. Built alongside the entries so the
/// two lists share indices with `Layout::sidebar_rows`.
enum SidebarAction {
    ToggleSection(usize),
    Go(String),
}

/// What the left mouse button is currently doing.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Divider { grab_offset: i32 },
    Scrollbar { side: Side, grab_offset: i32 },
}

struct AppState {
    renderer: Renderer,
    metrics: Metrics,
    dpi: u32,
    client: Rect,

    left: Pane,
    right: Pane,
    focused: Side,

    /// Divider position in client coordinates.
    split_x: i32,
    /// False shows one pane. The second pane keeps its state while hidden.
    dual: bool,
    sidebar_visible: bool,
    drives: Vec<fs::Drive>,
    places: Vec<Shortcut>,
    /// Collapsed state per sidebar section, indexed by SECTION_*.
    sections_collapsed: [bool; 2],
    /// Which pane's footer filter box is taking keystrokes, if any.
    filter_focus: Option<Side>,

    hover: Option<Hit>,
    drag: Drag,
    mouse_tracking: bool,

    theme: Theme,
    show_hidden: bool,
    /// Scrolling one pane scrolls the other to the same row.
    sync_scroll: bool,
    /// Mark entries that the other pane does not have.
    compare: bool,
    /// One directory watcher per pane, dropped (and cancelled) on navigation.
    watchers: [Option<Watcher>; 2],

    type_ahead: String,
    type_ahead_at: Option<Instant>,

    /// Set while a modal dialog owns input, so stray messages are ignored.
    modal: bool,
    status_override: Option<String>,
    /// Last title pushed to the window, so we only call SetWindowText on change.
    last_title: String,
}

impl AppState {
    fn new(dpi: u32) -> Result<Self> {
        let cfg = Config::load();
        let theme = if cfg.theme_dark {
            Theme::Dark
        } else {
            Theme::Light
        };
        let metrics = Metrics::for_dpi(dpi);
        let mut left = Pane::new();
        let mut right = Pane::new();
        left.set_row_height(metrics.row_h as u32);
        right.set_row_height(metrics.row_h as u32);
        left.set_show_hidden(cfg.show_hidden);
        right.set_show_hidden(cfg.show_hidden);
        Ok(Self {
            renderer: Renderer::new(theme, dpi)?,
            metrics,
            dpi,
            client: Rect::default(),
            left,
            right,
            focused: Side::Left,
            split_x: cfg.split_x,
            dual: cfg.dual,
            sidebar_visible: cfg.sidebar_visible,
            drives: fs::drives(),
            places: default_places(),
            sections_collapsed: [false, false],
            filter_focus: None,
            hover: None,
            drag: Drag::None,
            mouse_tracking: false,
            theme,
            show_hidden: cfg.show_hidden,
            sync_scroll: cfg.sync_scroll,
            compare: cfg.compare,
            watchers: [None, None],
            type_ahead: String::new(),
            type_ahead_at: None,
            modal: false,
            status_override: None,
            last_title: String::new(),
        })
    }

    fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }
    fn pane_mut(&mut self, side: Side) -> &mut Pane {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }
    fn focused_pane(&self) -> &Pane {
        self.pane(self.focused)
    }
    fn other_side(&self) -> Side {
        match self.focused {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Build the sidebar's rows and, in lockstep, what clicking each one does.
    /// Returning both from one place keeps the indices in sync with the
    /// rectangles the layout produces from the same list.
    fn sidebar_model(&self) -> (Vec<SidebarEntry>, Vec<SidebarAction>) {
        let mut entries = Vec::new();
        let mut actions = Vec::new();

        entries.push(SidebarEntry::Section {
            label: "Drives".into(),
            collapsed: self.sections_collapsed[SECTION_DRIVES],
        });
        actions.push(SidebarAction::ToggleSection(SECTION_DRIVES));
        if !self.sections_collapsed[SECTION_DRIVES] {
            for (i, d) in self.drives.iter().enumerate() {
                entries.push(SidebarEntry::Drive {
                    index: i,
                    used: d.used_fraction(),
                });
                actions.push(SidebarAction::Go(d.root.clone()));
            }
        }

        entries.push(SidebarEntry::Section {
            label: "Places".into(),
            collapsed: self.sections_collapsed[SECTION_PLACES],
        });
        actions.push(SidebarAction::ToggleSection(SECTION_PLACES));
        if !self.sections_collapsed[SECTION_PLACES] {
            for (i, sc) in self.places.iter().enumerate() {
                entries.push(SidebarEntry::Place { index: i });
                actions.push(SidebarAction::Go(sc.path.clone()));
            }
        }

        (entries, actions)
    }

    /// Recompute the frame. Cheap: the renderer memoises text measurement.
    fn layout(&self) -> Layout {
        let left_tabs: Vec<String> = self.left.tabs.iter().map(|t| t.label()).collect();
        let right_tabs: Vec<String> = self.right.tabs.iter().map(|t| t.label()).collect();
        let left_crumbs = fs::path_segments(self.left.current_path());
        let right_crumbs = fs::path_segments(self.right.current_path());

        let li = PaneInput {
            tab_labels: &left_tabs,
            crumbs: &left_crumbs,
            total_rows: self.left.list().total_rows(),
            scroll_offset: self.left.list().scroll_offset,
        };
        let ri = PaneInput {
            tab_labels: &right_tabs,
            crumbs: &right_crumbs,
            total_rows: self.right.list().total_rows(),
            scroll_offset: self.right.list().scroll_offset,
        };
        let (entries, _) = self.sidebar_model();
        Layout::compute(
            self.client,
            self.metrics,
            self.split_x,
            self.sidebar_visible,
            &entries,
            &li,
            if self.dual { Some(&ri) } else { None },
            &self.renderer,
        )
    }

    fn list_height(&self, side: Side) -> u32 {
        self.layout().pane(side).list.h.max(0) as u32
    }

    fn clamp_split(&mut self) {
        self.split_x =
            Layout::clamp_split(self.client, self.metrics, self.sidebar_visible, self.split_x);
    }

    fn config(&self, hwnd: HWND) -> Config {
        let mut c = Config {
            dual: self.dual,
            theme_dark: self.theme.is_dark(),
            sidebar_visible: self.sidebar_visible,
            show_hidden: self.show_hidden,
            split_x: self.split_x,
            sync_scroll: self.sync_scroll,
            compare: self.compare,
            ..Config::default()
        };
        // rcNormalPosition is the restored box even while maximised, which is
        // what we want to come back to after un-maximising.
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_ok() {
            let r = wp.rcNormalPosition;
            c.win_x = r.left;
            c.win_y = r.top;
            c.win_w = (r.right - r.left).max(400);
            c.win_h = (r.bottom - r.top).max(300);
            c.maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        }
        c
    }

    /// Point this pane's watcher at whatever it is now showing. Dropping the
    /// old watcher cancels its blocking read, so there is never more than one
    /// thread per pane.
    fn rewatch(&mut self, side: Side, hwnd: HWND) {
        let i = side_index(side);
        let path = self.pane(side).current_path().to_string();
        self.watchers[i] = None;
        self.watchers[i] = Watcher::start(hwnd, side, WM_APP_DIR_CHANGED, &path);
    }

    /// Keep both panes on the same row when sync scrolling is on.
    fn mirror_scroll(&mut self, from: Side) {
        if !self.sync_scroll || !self.dual {
            return;
        }
        let offset = self.pane(from).list().scroll_offset;
        let to = match from {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        };
        let h = self.list_height(to);
        self.pane_mut(to).list_mut().scroll_to(offset, h);
    }

    /// The right-hand text in a pane's footer.
    fn counts_text(&self, side: Side) -> String {
        let pane = self.pane(side);
        if side == self.focused {
            if let Some(s) = &self.status_override {
                return s.clone();
            }
        }
        let list = pane.list();
        if pane.active().loading && list.entries.is_empty() {
            return "Loading\u{2026}".to_string();
        }
        let shown = list.total_rows() as usize;
        let sel = list.selection_count();
        if sel > 0 {
            return format!(
                "{} selected  \u{00B7}  {}",
                sel,
                fs::format_size(list.selected_size())
            );
        }
        if list.is_filtered() {
            return format!("{} of {}", shown, list.unfiltered_count());
        }
        format!("{} item{}", shown, if shown == 1 { "" } else { "s" })
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    unsafe {
        // The shell interfaces we use (IFileOperation, SHGetFileInfoW, WIC) all
        // want an initialised apartment on the calling thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

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
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        CoUninitialize();
    }
    Ok(())
}

/// Match the title bar to the app's theme. Without this a dark app sits under a
/// light title bar, which is the first thing anyone notices.
fn apply_titlebar_theme(hwnd: HWND, theme: Theme) {
    let dark = BOOL::from(theme.is_dark());
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

// ---------------------------------------------------------------------------
// Async work
// ---------------------------------------------------------------------------

fn spawn_dir_load(hwnd: HWND, side: Side, req: LoadRequest) {
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let result = fs::list_dir(&req.path).map_err(|e| e.to_string());
        let payload = Box::new(DirLoaded { side, req, result });
        let raw = Box::into_raw(payload);
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_DIR_LOADED,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            // Window already gone: reclaim rather than leak.
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

fn start_load(hwnd: HWND, side: Side, req: Option<LoadRequest>) {
    if let Some(req) = req {
        spawn_dir_load(hwnd, side, req);
    }
}

fn spawn_op(hwnd: HWND, op: ops::Op) {
    let owner = ops::OwnerWindow(hwnd);
    ops::spawn(op, owner, move |result| {
        let raw = Box::into_raw(result);
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_OP_DONE,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

/// Walk each named folder on a worker thread and post the totals back.
fn spawn_dir_sizes(hwnd: HWND, side: Side, tab_id: u64, dir: String, names: Vec<String>) {
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
            side,
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
fn calculate_folder_sizes(state: &mut AppState, hwnd: HWND) {
    let side = state.focused;
    let pane = state.pane(side);
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
    let tab_id = state.pane(side).active_tab_id();
    spawn_dir_sizes(hwnd, side, tab_id, dir, names);
}

fn report_error(hwnd: HWND, title: &str, message: &str) {
    let t = wide(title);
    let m = wide(message);
    unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(m.as_ptr()),
            PCWSTR::from_raw(t.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

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
            let start = fs::current_directory().unwrap_or_else(|_| FALLBACK_PATH.to_string());
            let l = state.left.navigate(&start);
            let r = state.right.navigate(&start);
            spawn_dir_load(hwnd, Side::Left, l);
            spawn_dir_load(hwnd, Side::Right, r);
            Some(LRESULT(0))
        }

        WM_DESTROY => {
            // Drop the watchers first: their threads post to this window.
            state.watchers = [None, None];
            state.config(hwnd).save();
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }

        // We paint every pixel; erasing first would only flicker.
        WM_ERASEBKGND => Some(LRESULT(1)),

        WM_SIZE => {
            let w = loword(lparam.0 as u32).max(0);
            let h = hiword(lparam.0 as u32).max(0);
            // Only centre the divider when nothing was restored for it.
            let first = state.client.w == 0 && state.split_x == 0;
            state.client = Rect::new(0, 0, w, h);
            if first {
                // Start with the panes even.
                state.split_x = state.client.x
                    + if state.sidebar_visible {
                        state.metrics.sidebar_w
                    } else {
                        0
                    }
                    + (w - if state.sidebar_visible { state.metrics.sidebar_w } else { 0 }) / 2;
            }
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
            state.left.set_row_height(state.metrics.row_h as u32);
            state.right.set_row_height(state.metrics.row_h as u32);
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
                let over_divider = matches!(state.hover, Some(Hit::Divider))
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
            if let Hit::Tab(side, i) | Hit::TabClose(side, i) = state.layout().hit_test(x, y) {
                state.focused = side;
                let req = state.pane_mut(side).close_tab(i, FALLBACK_PATH);
                start_load(hwnd, side, req);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_XBUTTONDOWN => {
            // XBUTTON1 = back, XBUTTON2 = forward, as everywhere else.
            let button = hiword(wparam.0 as u32);
            let side = state.focused;
            let req = if button == XBUTTON1 as i32 {
                state.pane_mut(side).go_back()
            } else {
                state.pane_mut(side).go_forward()
            };
            start_load(hwnd, side, req);
            invalidate(hwnd);
            Some(LRESULT(1))
        }

        WM_MOUSEWHEEL => {
            let delta = hiword(wparam.0 as u32);
            let mut pt = POINT {
                x: loword(lparam.0 as u32),
                y: hiword(lparam.0 as u32),
            };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            let layout = state.layout();
            // Scroll whatever the pointer is over, which is what people expect,
            // without stealing keyboard focus from the other pane.
            let side = if layout.right.bounds.contains(pt.x, pt.y) {
                Side::Right
            } else if layout.left.bounds.contains(pt.x, pt.y) {
                Side::Left
            } else {
                state.focused
            };
            let lines = if delta > 0 { -3 } else { 3 };
            let h = layout.pane(side).list.h.max(0) as u32;
            state.pane_mut(side).list_mut().scroll_by(lines, h);
            state.mirror_scroll(side);
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

        WM_APP_DIR_LOADED => {
            let payload = unsafe { Box::from_raw(lparam.0 as *mut DirLoaded) };
            let side = payload.side;
            let h = state.list_height(side);
            let changed = state.pane_mut(side).finish_load(
                payload.req.tab_id,
                payload.req.generation,
                &payload.req.path,
                payload.req.kind,
                payload.result,
                h,
            );
            if changed {
                state.rewatch(side, hwnd);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_DIR_CHANGED => {
            // Coalesce: restart the timer so a burst of writes collapses into
            // a single reload once things settle.
            let side = side_from_index(wparam.0);
            let id = match side {
                Side::Left => TIMER_WATCH_LEFT,
                Side::Right => TIMER_WATCH_RIGHT,
            };
            unsafe { SetTimer(Some(hwnd), id, WATCH_DEBOUNCE_MS, None) };
            Some(LRESULT(0))
        }

        WM_TIMER => {
            let id = wparam.0;
            if id == TIMER_WATCH_LEFT || id == TIMER_WATCH_RIGHT {
                unsafe {
                    let _ = KillTimer(Some(hwnd), id);
                }
                let side = if id == TIMER_WATCH_LEFT {
                    Side::Left
                } else {
                    Side::Right
                };
                let req = state.pane_mut(side).refresh();
                start_load(hwnd, side, req);
                return Some(LRESULT(0));
            }
            None
        }

        WM_APP_SIZES_DONE => {
            let done = unsafe { Box::from_raw(lparam.0 as *mut SizesDone) };
            if let Some(tab) = state.pane_mut(done.side).tab_mut(done.tab_id) {
                tab.file_list.set_dir_sizes(done.sizes);
                invalidate(hwnd);
            }
            Some(LRESULT(0))
        }

        WM_APP_OP_DONE => {
            let result = unsafe { Box::from_raw(lparam.0 as *mut ops::OpResult) };
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
                for side in [Side::Left, Side::Right] {
                    if !pane::paths_equal(state.pane(side).current_path(), &dir) {
                        continue;
                    }
                    if let Some(name) = &created {
                        state.pane_mut(side).select_after_next_load(name);
                    }
                    let req = state.pane_mut(side).refresh();
                    start_load(hwnd, side, req);
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

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn paint(state: &mut AppState, hwnd: HWND) {
    if !state.renderer.has_target() {
        let _ = state
            .renderer
            .resize(hwnd, state.client.w.max(0) as u32, state.client.h.max(0) as u32);
    }
    let layout = state.layout();
    let (sidebar_entries, _) = state.sidebar_model();
    let current = state.focused_pane().current_path().to_string();
    let dragging = matches!(state.drag, Drag::Divider { .. });
    let hover = state.hover;
    let focused_side = state.focused;
    let filter_focus = state.filter_focus;
    let drives = state.drives.clone();
    let places: Vec<(String, String)> = state
        .places
        .iter()
        .map(|p| (p.label.clone(), p.path.clone()))
        .collect();
    let counts = [
        state.counts_text(Side::Left),
        state.counts_text(Side::Right),
    ];
    let sides: &[Side] = if state.dual {
        &[Side::Left, Side::Right]
    } else {
        &[Side::Left]
    };
    let comparing = state.compare && state.dual;
    let (left_names, right_names) = if comparing {
        (state.left.list().names(), state.right.list().names())
    } else {
        (HashSet::new(), HashSet::new())
    };
    update_title(state, hwnd);

    // Destructure so the renderer can be borrowed mutably while the panes are
    // borrowed immutably. Borrowing through `state` would conflict.
    let AppState {
        renderer,
        left,
        right,
        ..
    } = state;

    renderer.begin();
    renderer.draw_sidebar(
        &layout,
        &SidebarView {
            entries: &sidebar_entries,
            drives: &drives,
            places: &places,
            current_path: &current,
            hover,
        },
    );

    for &side in sides {
        let pane: &Pane = match side {
            Side::Left => left,
            Side::Right => right,
        };
        let tabs: Vec<String> = pane.tabs.iter().map(|t| t.label()).collect();
        let crumbs = fs::path_segments(pane.current_path());
        let view = PaneView {
            side,
            layout: layout.pane(side),
            tab_labels: &tabs,
            active_tab: pane.active_tab_index,
            crumbs: &crumbs,
            list: pane.list(),
            focused: focused_side == side,
            loading: pane.active().loading,
            error: pane.active().error.as_deref(),
            hover,
            can_back: pane.active().can_go_back(),
            can_forward: pane.active().can_go_forward(),
            can_up: fs::path_parent(pane.current_path()).is_some(),
            filter_focused: filter_focus == Some(side),
            counts: &counts[if side == Side::Left { 0 } else { 1 }],
            other_names: comparing.then(|| match side {
                Side::Left => &right_names,
                Side::Right => &left_names,
            }),
        };
        renderer.draw_pane(layout.metrics, &view);
    }

    renderer.draw_divider(&layout, hover == Some(Hit::Divider), dragging);

    if renderer.end().is_err() {
        // Device lost: drop everything and come back next frame.
        renderer.discard_target();
        invalidate(hwnd);
    }
}

/// The user's standard folders, for the sidebar's Places section.
/// Entries whose folder does not exist are dropped rather than shown broken.
fn default_places() -> Vec<Shortcut> {
    let Ok(home) = std::env::var("USERPROFILE") else {
        return Vec::new();
    };
    let mut out = vec![Shortcut {
        label: "Home".to_string(),
        path: home.clone(),
    }];
    for name in [
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Music",
        "Videos",
    ] {
        let path = fs::path_join(&home, name);
        if std::path::Path::new(&path).is_dir() {
            out.push(Shortcut {
                label: name.to_string(),
                path,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

fn on_mouse_move(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
    match state.drag {
        Drag::Divider { grab_offset } => {
            state.split_x = x - grab_offset;
            state.clamp_split();
            invalidate(hwnd);
            return;
        }
        Drag::Scrollbar { side, grab_offset } => {
            let layout = state.layout();
            let p = layout.pane(side);
            let total = state.pane(side).list().total_rows();
            let visible = p.visible_rows;
            let offset = layout::scroll_offset_for_thumb_y(
                p.scrollbar,
                layout.metrics,
                total,
                visible,
                y - grab_offset,
            );
            let h = p.list.h.max(0) as u32;
            state.pane_mut(side).list_mut().scroll_to(offset, h);
            invalidate(hwnd);
            return;
        }
        Drag::None => {}
    }

    // Resolve the row under the cursor so hovering a row highlights it.
    let layout = state.layout();
    let mut hit = layout.hit_test(x, y);
    if let Hit::ListBackground(side) = hit {
        let p = layout.pane(side);
        let list = state.pane(side).list();
        if let Some(row) = p.row_at(y, list.scroll_offset, layout.metrics.row_h, list.total_rows())
        {
            hit = Hit::Row(side, row);
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

fn on_left_down(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
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
        Hit::Divider => {
            state.drag = Drag::Divider {
                grab_offset: x - layout.divider.x,
            };
            unsafe { SetCapture(hwnd) };
        }

        Hit::Sidebar(i) => {
            let (_, actions) = state.sidebar_model();
            match actions.get(i) {
                Some(SidebarAction::ToggleSection(sec)) => {
                    state.sections_collapsed[*sec] = !state.sections_collapsed[*sec];
                }
                Some(SidebarAction::Go(path)) => {
                    let path = path.clone();
                    let side = state.focused;
                    let req = state.pane_mut(side).navigate(&path);
                    spawn_dir_load(hwnd, side, req);
                }
                None => {}
            }
        }

        Hit::SidebarChevron(i) => {
            let (_, actions) = state.sidebar_model();
            if let Some(SidebarAction::ToggleSection(sec)) = actions.get(i) {
                state.sections_collapsed[*sec] = !state.sections_collapsed[*sec];
            }
        }

        Hit::Nav(side, which) => {
            state.focused = side;
            let req = match which {
                NavButton::Back => state.pane_mut(side).go_back(),
                NavButton::Forward => state.pane_mut(side).go_forward(),
                NavButton::Up => state.pane_mut(side).navigate_up(),
            };
            start_load(hwnd, side, req);
        }

        Hit::Filter(side) => {
            state.focused = side;
            state.filter_focus = Some(side);
        }

        Hit::Tab(side, i) => {
            state.focused = side;
            let req = state.pane_mut(side).switch_tab(i);
            start_load(hwnd, side, req);
        }

        Hit::TabClose(side, i) => {
            state.focused = side;
            let req = state.pane_mut(side).close_tab(i, FALLBACK_PATH);
            start_load(hwnd, side, req);
        }

        Hit::NewTab(side) => {
            state.focused = side;
            let req = if state.pane(side).current_path().is_empty() {
                state.pane_mut(side).new_tab(FALLBACK_PATH)
            } else {
                state.pane_mut(side).duplicate_tab()
            };
            spawn_dir_load(hwnd, side, req);
        }

        Hit::Crumb(side, segment) => {
            state.focused = side;
            let segs = fs::path_segments(state.pane(side).current_path());
            let path = fs::path_from_segments(&segs, segment);
            if !path.is_empty() && !pane::paths_equal(&path, state.pane(side).current_path()) {
                let req = state.pane_mut(side).navigate(&path);
                spawn_dir_load(hwnd, side, req);
            }
        }

        Hit::Column(side, key) => {
            state.focused = side;
            state.pane_mut(side).list_mut().apply_sort(key);
        }

        Hit::Row(side, row) => {
            state.focused = side;
            let mode = select_mode();
            state.pane_mut(side).list_mut().select(row, mode);
        }

        Hit::ListBackground(side) => {
            state.focused = side;
            let p = layout.pane(side);
            let list = state.pane(side).list();
            let row = p.row_at(y, list.scroll_offset, layout.metrics.row_h, list.total_rows());
            match row {
                Some(row) => {
                    let mode = select_mode();
                    state.pane_mut(side).list_mut().select(row, mode);
                }
                None => {
                    if !ctrl_down() {
                        state.pane_mut(side).list_mut().clear_selection();
                    }
                }
            }
        }

        Hit::ScrollbarThumb(side) => {
            state.focused = side;
            let thumb = layout.pane(side).thumb;
            state.drag = Drag::Scrollbar {
                side,
                grab_offset: y - thumb.y,
            };
            unsafe { SetCapture(hwnd) };
        }

        Hit::ScrollbarTrack(side, t) => {
            state.focused = side;
            let p = layout.pane(side);
            let total = state.pane(side).list().total_rows();
            let visible = p.visible_rows;
            let offset = (t * total.saturating_sub(visible) as f32).round() as u32;
            let h = p.list.h.max(0) as u32;
            state.pane_mut(side).list_mut().scroll_to(offset, h);
        }

        Hit::Nothing => {}
    }
    invalidate(hwnd);
}

fn on_double_click(state: &mut AppState, hwnd: HWND, x: i32, y: i32) {
    let layout = state.layout();
    let hit = layout.hit_test(x, y);

    // Double-clicking the divider resets the split to even, like a window
    // splitter should.
    if hit == Hit::Divider {
        let body_left = layout.client.x
            + if state.sidebar_visible {
                state.metrics.sidebar_w
            } else {
                0
            };
        state.split_x = body_left + (layout.client.right() - body_left) / 2;
        state.clamp_split();
        invalidate(hwnd);
        return;
    }

    let side = match hit {
        Hit::Row(s, _) | Hit::ListBackground(s) => s,
        _ => return,
    };
    state.focused = side;
    let p = layout.pane(side);
    let list = state.pane(side).list();
    let Some(row) = p.row_at(y, list.scroll_offset, layout.metrics.row_h, list.total_rows()) else {
        return;
    };
    state.pane_mut(side).list_mut().select(row, SelectMode::Replace);
    activate_selection(state, hwnd, side);
}

/// Enter or double-click: open a folder in place, or hand a file to the shell.
fn activate_selection(state: &mut AppState, hwnd: HWND, side: Side) {
    let pane = state.pane(side);
    let Some(entry) = pane.list().cursor_entry() else {
        return;
    };
    let dir = pane.current_path().to_string();
    let target = fs::path_join(&dir, &entry.name);

    if entry.is_dir {
        let req = state.pane_mut(side).navigate(&target);
        spawn_dir_load(hwnd, side, req);
        invalidate(hwnd);
    } else if let Err(e) = ops::shell_open(hwnd, &target) {
        report_error(hwnd, "Cannot open file", &ops::format_hresult(&e));
    }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

fn on_key_down(state: &mut AppState, hwnd: HWND, vk: VIRTUAL_KEY) -> bool {
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
    let side = state.focused;
    let list_h = state.list_height(side);
    let ctrl = ctrl_down();
    let shift = shift_down();
    let alt = alt_down();
    let mode = select_mode();

    let mut handled = true;
    match vk {
        VK_UP | VK_DOWN => {
            let delta = if vk == VK_UP { -1 } else { 1 };
            if alt && vk == VK_UP {
                let req = state.pane_mut(side).navigate_up();
                start_load(hwnd, side, req);
            } else {
                let list = state.pane_mut(side).list_mut();
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
            let page = state.layout().pane(side).visible_rows.max(1) as i32;
            let delta = if vk == VK_PRIOR { -page } else { page };
            let list = state.pane_mut(side).list_mut();
            if let Some(n) = list.move_cursor(delta, mode) {
                list.ensure_visible(n, list_h);
            }
        }

        VK_HOME | VK_END => {
            let list = state.pane_mut(side).list_mut();
            let target = if vk == VK_HOME {
                0
            } else {
                list.total_rows().saturating_sub(1)
            };
            if let Some(n) = list.cursor_to(target, mode) {
                list.ensure_visible(n, list_h);
            }
        }

        VK_RETURN => activate_selection(state, hwnd, side),

        VK_BACK => {
            let req = state.pane_mut(side).navigate_up();
            start_load(hwnd, side, req);
        }

        VK_LEFT if alt => {
            let req = state.pane_mut(side).go_back();
            start_load(hwnd, side, req);
        }
        VK_RIGHT if alt => {
            let req = state.pane_mut(side).go_forward();
            start_load(hwnd, side, req);
        }

        VK_TAB if ctrl => {
            let delta = if shift { -1 } else { 1 };
            let req = state.pane_mut(side).next_tab(delta);
            start_load(hwnd, side, req);
        }
        VK_TAB => {
            state.focused = state.other_side();
        }

        VK_F2 => do_rename(state, hwnd),
        VK_F5 => {
            let req = state.pane_mut(side).refresh();
            start_load(hwnd, side, req);
        }
        VK_F6 => copy_or_move_to_other(state, hwnd, false),

        VK_DELETE => do_delete(state, hwnd, shift),

        VK_ESCAPE => {
            let list = state.pane_mut(side).list_mut();
            if list.is_filtered() {
                list.clear_filter();
            } else {
                list.clear_selection();
            }
            state.type_ahead.clear();
        }

        _ => {
            let c = vk.0 as u8 as char;
            match (ctrl, shift, c) {
                (true, false, 'A') => state.pane_mut(side).list_mut().select_all(),
                (true, false, 'C') => do_clipboard(state, hwnd, ops::DropEffect::Copy),
                (true, false, 'X') => do_clipboard(state, hwnd, ops::DropEffect::Move),
                (true, false, 'V') => do_paste(state, hwnd),
                (true, false, 'T') => {
                    let req = if state.pane(side).current_path().is_empty() {
                        state.pane_mut(side).new_tab(FALLBACK_PATH)
                    } else {
                        state.pane_mut(side).duplicate_tab()
                    };
                    spawn_dir_load(hwnd, side, req);
                }
                (true, false, 'W') => {
                    let req = state.pane_mut(side).close_active_tab(FALLBACK_PATH);
                    start_load(hwnd, side, req);
                }
                (true, false, 'H') => toggle_hidden(state, hwnd),
                (true, false, 'F') => {
                    // Ctrl+F focuses the pane filter, as it does everywhere else.
                    state.filter_focus = Some(side);
                }
                (true, false, 'B') => {
                    state.sidebar_visible = !state.sidebar_visible;
                    state.clamp_split();
                }
                (true, false, '1') => set_dual(state, false),
                (true, false, '2') => set_dual(state, true),
                (true, false, 'R') => {
                    let req = state.pane_mut(side).refresh();
                    start_load(hwnd, side, req);
                }
                (true, true, 'N') => do_new_folder(state, hwnd),
                (true, true, 'C') => copy_or_move_to_other(state, hwnd, false),
                (true, true, 'M') => copy_or_move_to_other(state, hwnd, true),
                (true, true, 'D') => toggle_theme(state, hwnd),
                _ => handled = false,
            }
        }
    }

    if handled {
        invalidate(hwnd);
    }
    handled
}

fn on_type_ahead(state: &mut AppState, hwnd: HWND, c: char) {
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

    let side = state.focused;
    let list_h = state.list_height(side);
    let prefix = state.type_ahead.clone();
    let list = state.pane_mut(side).list_mut();
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

fn toggle_hidden(state: &mut AppState, hwnd: HWND) {
    state.show_hidden = !state.show_hidden;
    // FileList keeps the unfiltered entries, so this is instant: no reload.
    state.left.set_show_hidden(state.show_hidden);
    state.right.set_show_hidden(state.show_hidden);
    invalidate(hwnd);
}

/// Show one pane or two. Collapsing while the right pane is focused swaps the
/// panes first, so the folder you were looking at is the one that stays on screen.
fn set_dual(state: &mut AppState, dual: bool) {
    if state.dual == dual {
        return;
    }
    if !dual && state.focused == Side::Right {
        std::mem::swap(&mut state.left, &mut state.right);
    }
    state.dual = dual;
    state.focused = Side::Left;
    state.clamp_split();
}

/// Keep the title bar showing the focused folder. Called from paint, which is
/// the one place that always runs after anything that could change it.
fn update_title(state: &mut AppState, hwnd: HWND) {
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

fn toggle_theme(state: &mut AppState, hwnd: HWND) {
    state.theme = state.theme.toggled();
    state.renderer.set_theme(state.theme);
    apply_titlebar_theme(hwnd, state.theme);
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

fn do_clipboard(state: &mut AppState, hwnd: HWND, effect: ops::DropEffect) {
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

fn do_paste(state: &mut AppState, hwnd: HWND) {
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

fn copy_or_move_to_other(state: &mut AppState, hwnd: HWND, move_it: bool) {
    if !state.dual {
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

fn do_delete(state: &mut AppState, hwnd: HWND, permanent: bool) {
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

fn do_rename(state: &mut AppState, hwnd: HWND) {
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

fn do_new_folder(state: &mut AppState, hwnd: HWND) {
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

fn show_context_menu(state: &mut AppState, hwnd: HWND, screen_x: i32, screen_y: i32) {
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
    let side = match hit {
        Hit::Row(s, _) | Hit::ListBackground(s) | Hit::Column(s, _) => s,
        _ => state.focused,
    };
    state.focused = side;

    // Right-clicking an unselected row selects it first, as Explorer does.
    if let Hit::ListBackground(_) | Hit::Row(..) = hit {
        let p = layout.pane(side);
        let list = state.pane(side).list();
        if let Some(row) = p.row_at(
            client.y,
            list.scroll_offset,
            layout.metrics.row_h,
            list.total_rows(),
        ) {
            if !state.pane(side).list().is_selected(row) {
                state.pane_mut(side).list_mut().select(row, SelectMode::Replace);
            }
        }
    }
    invalidate(hwnd);

    let has_selection = state.pane(side).list().selection_count() > 0;
    let single = state.pane(side).list().selection_count() == 1;
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
        sep();
        add(CMD_PROPERTIES, "Properties", single);

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
        run_command(state, hwnd, cmd.0 as usize);
    }
}

fn run_command(state: &mut AppState, hwnd: HWND, cmd: usize) {
    let side = state.focused;
    match cmd {
        CMD_OPEN => activate_selection(state, hwnd, side),
        CMD_OPEN_NEW_TAB => {
            let pane = state.pane(side);
            if let Some(entry) = pane.list().cursor_entry() {
                if entry.is_dir {
                    let path = fs::path_join(pane.current_path(), &entry.name);
                    let req = state.pane_mut(side).new_tab(&path);
                    spawn_dir_load(hwnd, side, req);
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
            let req = state.pane_mut(side).refresh();
            start_load(hwnd, side, req);
        }
        CMD_CALC_SIZES => calculate_folder_sizes(state, hwnd),
        CMD_SYNC_SCROLL => {
            state.sync_scroll = !state.sync_scroll;
            state.mirror_scroll(side);
        }
        CMD_COMPARE => state.compare = !state.compare,
        CMD_PROPERTIES => {
            let paths = state.pane(side).selected_paths();
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

// ---------------------------------------------------------------------------
// Modal text prompt
//
// The previous rename dialog kept its result in a `static mut`, never disabled
// its parent (so it was not actually modal and could be opened recursively),
// and ran a nested message loop that swallowed WM_QUIT â€” meaning the app could
// not be closed while it was open. This version keeps per-dialog state in the
// window, disables the owner for real, and re-posts WM_QUIT on the way out.
// ---------------------------------------------------------------------------

const IDC_PROMPT_EDIT: i32 = 1001;
const IDC_PROMPT_OK: i32 = 1002;
const IDC_PROMPT_CANCEL: i32 = 1003;
const PROMPT_CLASS: &str = "FileXplorerPromptDlg";
/// EM_SETSEL. Declared here rather than pulling in all of Win32_UI_Controls
/// for one constant.
const EM_SETSEL: u32 = 0x00B1;

struct PromptState {
    label: String,
    initial: String,
    result: Option<String>,
    accepted: bool,
    dpi: u32,
}

fn prompt_text(
    hwnd: HWND,
    state: &mut AppState,
    title: &str,
    label: &str,
    initial: &str,
) -> Option<String> {
    if state.modal {
        return None; // Refuse to stack dialogs.
    }
    state.modal = true;
    let out = unsafe { prompt_text_impl(hwnd, title, label, initial, state.dpi) };
    state.modal = false;
    out
}

unsafe fn prompt_text_impl(
    parent: HWND,
    title: &str,
    label: &str,
    initial: &str,
    dpi: u32,
) -> Option<String> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = wide(PROMPT_CLASS);

    // Registering twice is harmless: the second call fails and we carry on.
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(prompt_proc),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
        hbrBackground: HBRUSH(((COLOR_WINDOW.0 + 1) as isize) as *mut _),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);

    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale).round() as i32;
    let (w, h) = (s(380.0), s(160.0));

    // Centre on the parent.
    let mut pr = RECT::default();
    let _ = GetWindowRect(parent, &mut pr);
    let x = pr.left + ((pr.right - pr.left) - w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - h) / 3;

    let st = Box::into_raw(Box::new(PromptState {
        label: label.to_string(),
        initial: initial.to_string(),
        result: None,
        accepted: false,
        dpi,
    }));

    let title_w = wide(title);
    let dlg = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title_w.as_ptr()),
        WS_POPUPWINDOW | WS_CAPTION,
        x,
        y,
        w,
        h,
        Some(parent),
        None,
        Some(instance.into()),
        Some(st as *const _),
    );

    let Ok(dlg) = dlg else {
        drop(Box::from_raw(st));
        return None;
    };

    // Real modality: the owner stops accepting input until we are done.
    let _ = EnableWindow(parent, false);
    let _ = ShowWindow(dlg, SW_SHOW);
    let _ = SetFocus(Some(dlg));

    let mut quit_code: Option<i32> = None;
    let mut msg = MSG::default();
    while IsWindow(Some(dlg)).as_bool() {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if r.0 == -1 {
            break;
        }
        if r.0 == 0 {
            // WM_QUIT arrived while we were modal. Remember it and re-post
            // after unwinding so the app still exits.
            quit_code = Some(msg.wParam.0 as i32);
            break;
        }
        if !IsDialogMessageW(dlg, &mut msg).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = EnableWindow(parent, true);
    let _ = SetActiveWindow(parent);
    if IsWindow(Some(dlg)).as_bool() {
        let _ = DestroyWindow(dlg);
    }

    let st = Box::from_raw(st);
    if let Some(code) = quit_code {
        PostQuitMessage(code);
    }
    if st.accepted {
        st.result.clone()
    } else {
        None
    }
}

extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let st = cs.lpCreateParams as *mut PromptState;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, st as isize);
                let state = &*st;
                let scale = state.dpi as f32 / 96.0;
                let s = |v: f32| (v * scale).round() as i32;
                let instance = cs.hInstance;

                let static_class = wide("Static");
                let label = wide(&state.label);
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(static_class.as_ptr()),
                    PCWSTR::from_raw(label.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    s(14.0),
                    s(12.0),
                    s(340.0),
                    s(18.0),
                    Some(hwnd),
                    None,
                    Some(instance),
                    None,
                );

                let edit_class = wide("Edit");
                let edit = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(edit_class.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    s(14.0),
                    s(34.0),
                    s(346.0),
                    s(26.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_EDIT as *mut _)),
                    Some(instance),
                    None,
                );

                let button_class = wide("Button");
                let ok = wide("OK");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button_class.as_ptr()),
                    PCWSTR::from_raw(ok.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                    s(174.0),
                    s(74.0),
                    s(90.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_OK as *mut _)),
                    Some(instance),
                    None,
                );

                let cancel = wide("Cancel");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button_class.as_ptr()),
                    PCWSTR::from_raw(cancel.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    s(270.0),
                    s(74.0),
                    s(90.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_CANCEL as *mut _)),
                    Some(instance),
                    None,
                );

                if let Ok(edit) = edit {
                    let initial = wide(&state.initial);
                    let _ = SetWindowTextW(edit, PCWSTR::from_raw(initial.as_ptr()));
                    // Preselect the stem so typing replaces the name but keeps
                    // the extension a keystroke away.
                    let stem_len = state
                        .initial
                        .rfind('.')
                        .filter(|i| *i > 0)
                        .unwrap_or(state.initial.chars().count());
                    SendMessageW(
                        edit,
                        EM_SETSEL,
                        Some(WPARAM(0)),
                        Some(LPARAM(stem_len as isize)),
                    );
                    let _ = SetFocus(Some(edit));
                }
                LRESULT(0)
            }

            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState;
                if id == IDC_PROMPT_OK && !st.is_null() {
                    if let Ok(edit) = GetDlgItem(Some(hwnd), IDC_PROMPT_EDIT) {
                        // Ask how long the text is instead of assuming MAX_PATH,
                        // which silently truncated long names before.
                        let len = GetWindowTextLengthW(edit).max(0) as usize;
                        let mut buf = vec![0u16; len + 1];
                        let n = GetWindowTextW(edit, &mut buf);
                        let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
                        (*st).result = Some(text.trim().to_string());
                        (*st).accepted = true;
                    }
                    let _ = DestroyWindow(hwnd);
                } else if id == IDC_PROMPT_CANCEL {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            // The owner frees the state; the dialog only clears its pointer.
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

