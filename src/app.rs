// Application state, and the small helpers every other module needs.
//
// `AppState` is created in WM_NCCREATE and lives in the window's user data for
// as long as the window does. Everything a message handler needs hangs off it,
// so handlers stay free functions taking `&mut AppState`.

use std::time::Instant;

use windows::core::{Result, BOOL, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::file_list::SelectMode;
use crate::fs;
use crate::layout::{Hit, Layout, Metrics, PaneId, PaneInput, Rect, SidebarEntry, MAX_PANES};
use crate::ops;
use crate::pane::{LoadRequest, Pane};
use crate::renderer::Renderer;
use crate::search;
use crate::theme::Theme;
use crate::watch::Watcher;
use crate::dnd;


/// A finished directory read, posted back from a worker thread.
pub const WM_APP_DIR_LOADED: u32 = WM_APP + 1;
/// A finished file operation, posted back from a worker thread.
pub const WM_APP_OP_DONE: u32 = WM_APP + 2;
/// Calculated folder sizes, posted back from a worker thread.
pub const WM_APP_SIZES_DONE: u32 = WM_APP + 3;
/// A watched directory changed on disk. wParam is the pane index.
pub const WM_APP_DIR_CHANGED: u32 = WM_APP + 4;
/// A batch of search results, posted back from a worker thread.
pub const WM_APP_SEARCH_BATCH: u32 = WM_APP + 5;
/// Files were dropped on the window. lParam is a boxed `dnd::Dropped`.
pub const WM_APP_DROPPED: u32 = WM_APP + 6;

/// wParam on WM_APP_OP_DONE: this operation was itself an undo, so its own
/// inverse must not go back on the stack.
pub const OP_WAS_UNDO: usize = 1;
/// Far more than anyone reaches for, and small enough that the paths held by
/// a forgotten stack never add up to anything.
pub const UNDO_DEPTH: usize = 32;

/// How far the mouse must move with the button down before it counts as a drag
/// rather than a sloppy click.
pub const DRAG_THRESHOLD: i32 = 5;

/// Timer ids for coalescing change notifications, one per pane.
/// One coalescing timer per pane: id = TIMER_WATCH_BASE + pane index.
pub const TIMER_WATCH_BASE: usize = 1;
/// A burst of writes (an unzip, a build) should cost one reload, not hundreds.
pub const WATCH_DEBOUNCE_MS: u32 = 250;

pub const FALLBACK_PATH: &str = "C:\\";
/// How long a type-to-select prefix stays alive between keystrokes.
pub const TYPE_AHEAD_RESET_MS: u128 = 900;

/// WM_MOUSELEAVE is not re-exported by the windows crate. It must be a real
/// `const` and not a plain binding, or the match arm below becomes an
/// irrefutable pattern that swallows every message after it.
pub const WM_MOUSELEAVE: u32 = 0x02A3;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn loword(v: u32) -> i32 {
    (v & 0xFFFF) as u16 as i16 as i32
}
pub fn hiword(v: u32) -> i32 {
    ((v >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// Client coordinates from an LPARAM, sign-extended.
///
/// The old code masked with 0xFFFF and stored the result in a u32, so a drag
/// that left the window on the left or top produced coordinates near 65535.
pub fn mouse_pos(lparam: LPARAM) -> (i32, i32) {
    let v = lparam.0 as u32;
    (loword(v), hiword(v))
}

pub fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}
pub fn ctrl_down() -> bool {
    key_down(VK_CONTROL)
}
pub fn shift_down() -> bool {
    key_down(VK_SHIFT)
}
pub fn alt_down() -> bool {
    key_down(VK_MENU)
}

pub fn select_mode() -> SelectMode {
    if shift_down() {
        SelectMode::Range
    } else if ctrl_down() {
        SelectMode::Toggle
    } else {
        SelectMode::Replace
    }
}

pub struct SearchBatch {
    pub pid: PaneId,
    pub tab_id: u64,
    pub batch: search::Batch,
}

pub struct SizesDone {
    pub pid: PaneId,
    pub tab_id: u64,
    pub sizes: Vec<(String, u64)>,
}

pub struct DirLoaded {
    pub pid: PaneId,
    pub req: LoadRequest,
    pub result: std::result::Result<Vec<fs::FileEntry>, String>,
}

/// A named location in the sidebar's Places section.
pub struct Shortcut {
    pub label: String,
    pub path: String,
}

/// Which sidebar section a header row belongs to.
pub const SECTION_DRIVES: usize = 0;
pub const SECTION_PLACES: usize = 1;

/// What clicking a sidebar row should do. Built alongside the entries so the
/// two lists share indices with `Layout::sidebar_rows`.
pub enum SidebarAction {
    ToggleSection(usize),
    Go(String),
}

/// What the left mouse button is currently doing.
#[derive(Clone, Copy, PartialEq)]
pub enum Drag {
    None,
    Divider { index: usize, grab_offset: i32 },
    Scrollbar { pid: PaneId, grab_offset: i32 },
}

pub struct AppState {
    pub renderer: Renderer,
    pub metrics: Metrics,
    pub dpi: u32,
    pub client: Rect,

    /// Always `MAX_PANES` long. Panes past `pane_count` are off screen but
    /// keep their tabs and scroll, so Ctrl+1 then Ctrl+2 comes back to what
    /// was there.
    pub panes: Vec<Pane>,
    pub pane_count: usize,
    pub focused: PaneId,

    /// One fraction of the body width per divider; `pane_count - 1` of them.
    pub splits: Vec<f32>,
    pub sidebar_visible: bool,
    /// The settings as they were read at startup. Only the saved tab list is
    /// still wanted after `new`, and only until WM_CREATE has restored it.
    pub session: Config,
    pub drives: Vec<fs::Drive>,
    pub places: Vec<Shortcut>,
    /// Collapsed state per sidebar section, indexed by SECTION_*.
    pub sections_collapsed: [bool; 2],
    /// Which pane's footer filter box is taking keystrokes, if any.
    pub filter_focus: Option<PaneId>,

    pub hover: Option<Hit>,
    pub drag: Drag,
    pub mouse_tracking: bool,

    pub theme: Theme,
    pub show_hidden: bool,
    /// Scrolling one pane scrolls the other to the same row.
    pub sync_scroll: bool,
    /// Mark entries that the other pane does not have.
    pub compare: bool,
    /// One directory watcher per pane, dropped (and cancelled) on navigation.
    pub watchers: Vec<Option<Watcher>>,
    /// In-flight search per pane, cancelled on drop.
    pub searches: Vec<Option<search::Search>>,
    /// Drop-target registration; revoked when dropped.
    pub drop_target: Option<dnd::Registration>,
    /// Where the left button went down on a selected row, if a drag might start.
    pub drag_origin: Option<(i32, i32)>,

    /// Inverse operations, newest last. See `ops::Op::inverse` for what can
    /// and cannot be taken back.
    pub undo_stack: Vec<ops::Op>,

    pub type_ahead: String,
    pub type_ahead_at: Option<Instant>,

    /// Set while a modal dialog owns input, so stray messages are ignored.
    pub modal: bool,
    pub status_override: Option<String>,
    /// Last title pushed to the window, so we only call SetWindowText on change.
    pub last_title: String,
}

impl AppState {
    pub fn new(dpi: u32) -> Result<Self> {
        let cfg = Config::load();
        let theme = if cfg.theme_dark {
            Theme::Dark
        } else {
            Theme::Light
        };
        let metrics = Metrics::for_dpi(dpi);
        let panes: Vec<Pane> = (0..MAX_PANES)
            .map(|_| {
                let mut p = Pane::new();
                p.set_row_height(metrics.row_h as u32);
                p.set_show_hidden(cfg.show_hidden);
                p
            })
            .collect();
        Ok(Self {
            renderer: Renderer::new(theme, dpi)?,
            metrics,
            dpi,
            client: Rect::default(),
            panes,
            pane_count: cfg.pane_count.clamp(1, MAX_PANES),
            focused: PaneId(0),
            splits: cfg.splits.clone(),
            sidebar_visible: cfg.sidebar_visible,
            session: cfg.clone(),
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
            watchers: (0..MAX_PANES).map(|_| None).collect(),
            searches: (0..MAX_PANES).map(|_| None).collect(),
            drop_target: None,
            drag_origin: None,
            undo_stack: Vec::new(),
            type_ahead: String::new(),
            type_ahead_at: None,
            modal: false,
            status_override: None,
            last_title: String::new(),
        })
    }

    pub fn pane(&self, pid: PaneId) -> &Pane {
        &self.panes[pid.0.min(MAX_PANES - 1)]
    }
    pub fn pane_mut(&mut self, pid: PaneId) -> &mut Pane {
        &mut self.panes[pid.0.min(MAX_PANES - 1)]
    }
    pub fn focused_pane(&self) -> &Pane {
        self.pane(self.focused)
    }
    /// Every pane currently on screen, left to right.
    pub fn visible(&self) -> impl Iterator<Item = PaneId> {
        (0..self.pane_count).map(PaneId)
    }
    /// The next pane to the right, wrapping. With two panes this is "the other
    /// one", which is what every copy-to-other-pane command means.
    pub fn other_side(&self) -> PaneId {
        PaneId((self.focused.0 + 1) % self.pane_count.max(1))
    }

    /// Build the sidebar's rows and, in lockstep, what clicking each one does.
    /// Returning both from one place keeps the indices in sync with the
    /// rectangles the layout produces from the same list.
    pub fn sidebar_model(&self) -> (Vec<SidebarEntry>, Vec<SidebarAction>) {
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
    pub fn layout(&self) -> Layout {
        // Labels and crumbs have to outlive the PaneInputs that borrow them.
        let tabs: Vec<Vec<String>> = self
            .visible()
            .map(|p| self.pane(p).tabs.iter().map(|t| t.label()).collect())
            .collect();
        let crumbs: Vec<Vec<String>> = self
            .visible()
            .map(|p| fs::path_segments(self.pane(p).current_path()))
            .collect();
        let inputs: Vec<PaneInput> = self
            .visible()
            .map(|p| PaneInput {
                tab_labels: &tabs[p.0],
                crumbs: &crumbs[p.0],
                total_rows: self.pane(p).list().total_rows(),
                scroll_offset: self.pane(p).list().scroll_offset,
            })
            .collect();
        let (entries, _) = self.sidebar_model();
        Layout::compute(
            self.client,
            self.metrics,
            &self.splits,
            self.sidebar_visible,
            &entries,
            &inputs,
            &self.renderer,
        )
    }

    pub fn list_height(&self, pid: PaneId) -> u32 {
        self.layout().pane(pid).list.h.max(0) as u32
    }

    pub fn clamp_split(&mut self) {
        self.splits = Layout::clamp_splits(
            self.client,
            self.metrics,
            self.sidebar_visible,
            self.pane_count,
            &self.splits,
        );
    }

    /// Show `n` panes. Shrinking while a doomed pane is focused moves it into
    /// range first, so the folder you were looking at is the one that stays.
    pub fn set_pane_count(&mut self, n: usize) {
        let n = n.clamp(1, MAX_PANES);
        if n == self.pane_count {
            return;
        }
        if self.focused.0 >= n {
            self.panes.swap(self.focused.0, n - 1);
            self.watchers.swap(self.focused.0, n - 1);
            self.searches.swap(self.focused.0, n - 1);
            self.focused = PaneId(n - 1);
        }
        self.pane_count = n;
        // A different count means different dividers; even shares is the only
        // sane starting point, and the clamp would force it anyway.
        self.splits = Vec::new();
        self.clamp_split();
    }

    pub fn config(&self, hwnd: HWND) -> Config {
        let mut c = Config {
            pane_count: self.pane_count,
            theme_dark: self.theme.is_dark(),
            sidebar_visible: self.sidebar_visible,
            show_hidden: self.show_hidden,
            splits: self.splits.clone(),
            sync_scroll: self.sync_scroll,
            compare: self.compare,
            tabs: self.panes.iter().map(|p| p.tab_paths()).collect(),
            active_tab: self.panes.iter().map(|p| p.active_tab_index).collect(),
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
    pub fn rewatch(&mut self, pid: PaneId, hwnd: HWND) {
        let i = pid.0;
        let path = self.pane(pid).current_path().to_string();
        self.watchers[i] = None;
        self.watchers[i] = Watcher::start(hwnd, i, WM_APP_DIR_CHANGED, &path);
    }

    /// Keep every visible pane on the same row when sync scrolling is on.
    pub fn mirror_scroll(&mut self, from: PaneId) {
        if !self.sync_scroll || self.pane_count < 2 {
            return;
        }
        let offset = self.pane(from).list().scroll_offset;
        for to in self.visible().collect::<Vec<_>>() {
            if to == from {
                continue;
            }
            let h = self.list_height(to);
            self.pane_mut(to).list_mut().scroll_to(offset, h);
        }
    }

    /// The right-hand text in a pane's footer.
    pub fn counts_text(&self, pid: PaneId) -> String {
        let pane = self.pane(pid);
        if pid == self.focused {
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
// Chrome
// ---------------------------------------------------------------------------

/// Match the title bar to the app's theme. Without this a dark app sits under a
/// light title bar, which is the first thing anyone notices.
pub fn apply_titlebar_theme(hwnd: HWND, theme: Theme) {
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

pub fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub fn report_error(hwnd: HWND, title: &str, message: &str) {
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
// Async work
// ---------------------------------------------------------------------------

pub fn spawn_dir_load(hwnd: HWND, pid: PaneId, req: LoadRequest) {
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let result = fs::list_dir(&req.path).map_err(|e| e.to_string());
        let payload = Box::new(DirLoaded { pid, req, result });
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

pub fn start_load(hwnd: HWND, pid: PaneId, req: Option<LoadRequest>) {
    if let Some(req) = req {
        spawn_dir_load(hwnd, pid, req);
    }
}

pub fn spawn_op(hwnd: HWND, op: ops::Op) {
    spawn_op_tagged(hwnd, op, 0)
}

/// `tag` rides back on the completion message. `OP_WAS_UNDO` marks an
/// operation that came off the undo stack, which must not push its own
/// inverse back on: that would make Ctrl+Z a toggle.
pub fn spawn_op_tagged(hwnd: HWND, op: ops::Op, tag: usize) {
    let owner = ops::OwnerWindow(hwnd);
    ops::spawn(op, owner, move |result| {
        let raw = Box::into_raw(result);
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_OP_DONE,
                WPARAM(tag),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}


/// The user's standard folders, for the sidebar's Places section.
/// Entries whose folder does not exist are dropped rather than shown broken.
pub fn default_places() -> Vec<Shortcut> {
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
