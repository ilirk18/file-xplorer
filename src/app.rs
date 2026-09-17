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
use crate::file_list::{SelectMode, SortKey, SortOrder};
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

/// A background job finished. lParam is a boxed `TaskDone`.
pub const WM_APP_TASK_DONE: u32 = WM_APP + 7;
/// A content comparison finished. lParam is a boxed `DiffDone`.
pub const WM_APP_DIFF_DONE: u32 = WM_APP + 8;

/// A worker finished building an inspector preview. `lparam` owns a boxed
/// `(key, Preview)`; the handler takes it or leaks the bitmap inside it.
pub const WM_APP_PREVIEW_READY: u32 = WM_APP + 9;

/// A worker finished one grid cell's image. `lparam` owns a boxed
/// `(key, Option<HBITMAP>)`; the handler takes it or leaks the bitmap.
pub const WM_APP_THUMB_READY: u32 = WM_APP + 10;

/// Periodic settings save. Writing only on exit means a crash — or being
/// killed from a terminal — loses the session, which is exactly when you
/// most want it back.
pub const TIMER_AUTOSAVE: usize = 90;
pub const AUTOSAVE_MS: u32 = 4_000;

/// What a background job reports when it is done.
pub struct TaskDone {
    /// Shown in the footer. Empty clears it.
    pub message: String,
    pub error: Option<String>,
    /// Directories whose listings this may have changed.
    pub refresh: Vec<String>,
}

/// Names whose contents differ from the pane next door.
pub struct DiffDone {
    pub pid: PaneId,
    pub tab_id: u64,
    pub differing: Vec<String>,
}

/// Run `f` on a worker thread and post its result back.
pub fn spawn_task<F>(hwnd: HWND, f: F)
where
    F: FnOnce() -> TaskDone + Send + 'static,
{
    let owner = ops::OwnerWindow(hwnd);
    std::thread::spawn(move || {
        let raw = Box::into_raw(Box::new(f()));
        let posted = unsafe {
            PostMessageW(
                Some(owner.hwnd()),
                WM_APP_TASK_DONE,
                WPARAM(0),
                LPARAM(raw as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

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
pub const SECTION_TREE: usize = 2;

/// What clicking a sidebar row should do. Built alongside the entries so the
/// two lists share indices with `Layout::sidebar_rows`.
pub enum SidebarAction {
    ToggleSection(usize),
    /// Go here. On a tree row a click on the chevron expands instead, which is
    /// why this needs no separate "toggle" action.
    Go(String),
}

/// What the left mouse button is currently doing.
#[derive(Clone, Copy, PartialEq)]
pub enum Drag {
    None,
    Divider { index: usize, grab_offset: i32 },
    /// Resizing a column by its left edge.
    Column { key: crate::file_list::SortKey, start_x: i32, start_w: i32 },
    Scrollbar { pid: PaneId, grab_offset: i32 },
    /// Rubber-band selection, in client pixels.
    ///
    /// ponytail: replaces the selection; no Ctrl+band to add to it. That would
    /// mean carrying the starting selection through the drag, and `Drag` is
    /// `Copy` precisely because nothing in it owns anything.
    Band {
        pid: PaneId,
        origin: (i32, i32),
        cursor: (i32, i32),
    },
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
    pub sections_collapsed: [bool; 3],
    /// How far the sidebar is scrolled. A folder tree outgrows the window
    /// almost immediately, so it has to scroll.
    pub sidebar_scroll: i32,
    /// Folders the user pinned into Places.
    pub pins: Vec<String>,
    /// Type, Size and Date column widths in DIPs. Kept here rather than in
    /// `Metrics` because they are a preference, not a measurement — `metrics`
    /// holds the scaled copy the layout reads.
    pub col_widths: [i32; 3],
    /// The settings as last written, so the autosave only writes on a change.
    pub last_saved: Config,
    /// A tab being dragged: which pane, which index, and where it started.
    pub tab_drag: Option<(PaneId, usize, i32, i32)>,
    /// The sidebar's folder tree.
    pub tree: crate::tree::Tree,
    /// Its rows, rebuilt whenever the tree changes rather than on every
    /// layout pass: `layout()` runs on every mouse move.
    pub tree_rows: Vec<crate::tree::TreeRow>,
    /// Which pane's footer filter box is taking keystrokes, if any.
    pub filter_focus: Option<PaneId>,
    /// Which chord runs which command. Seeded from the command table's own
    /// default shortcuts, then overridden by whatever the settings file says.
    pub bindings: crate::keys::Bindings,
    /// Shell images for the icon view's cells, and the keys being fetched.
    pub thumbs: crate::preview::ThumbCache,
    thumbs_pending: std::collections::HashSet<String>,
    /// Icon view rather than the details list, in every pane. Per-pane would
    /// mean a per-pane array in the config like `sort`; nothing has asked.
    pub grid: bool,
    /// Is the inspector panel on the right showing?
    pub inspector: bool,
    /// What the inspector is showing, keyed by path and modified time so an
    /// edited file re-previews rather than showing a stale thumbnail.
    pub preview: Option<(String, crate::preview::Preview)>,
    /// The key a worker is currently fetching, so one selection does not
    /// spawn a thread per repaint.
    pub preview_pending: Option<String>,
    /// The inline rename editor, while a name is being typed over.
    pub rename: Option<crate::rename::InlineRename>,
    /// Folders visited, most recent first, offered by "Recent folders".
    pub recent: Vec<String>,
    /// The sort each folder was last given, for this session only. Keyed by
    /// lowercased path, because Windows paths are case-insensitive.
    ///
    /// ponytail: not persisted. Add `foldersort=` lines to the config if
    /// wanting it across restarts; the session map is where the value is.
    pub folder_sort: std::collections::HashMap<String, (SortKey, SortOrder)>,

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
            // The tree starts collapsed: it is the one section that costs a
            // directory read to show.
            sections_collapsed: [false, false, true],
            sidebar_scroll: 0,
            pins: cfg.pins.clone(),
            col_widths: cfg.col_widths,
            last_saved: cfg.clone(),
            tab_drag: None,
            tree: crate::tree::Tree::default(),
            tree_rows: Vec::new(),
            filter_focus: None,
            bindings: {
                let mut b = crate::keys::Bindings::from_defaults(
                    &crate::commands::default_bindings(),
                );
                for (chord, label) in &cfg.binds {
                    // A label the table no longer has is a binding for a
                    // command that no longer exists: drop it quietly rather
                    // than refuse to start.
                    if let Some(id) = crate::commands::id_for_label(label) {
                        match crate::keys::Chord::parse(chord) {
                            Some(c) => b.set(c, id),
                            // An empty chord is how "no shortcut" is recorded.
                            None => b.clear(id),
                        }
                    }
                }
                b
            },
            thumbs: Default::default(),
            thumbs_pending: Default::default(),
            grid: cfg.grid,
            inspector: cfg.inspector,
            preview: None,
            preview_pending: None,
            rename: None,
            recent: cfg.recent.clone(),
            folder_sort: std::collections::HashMap::new(),
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

        for (i, _) in self.pins.iter().enumerate() {
            if self.sections_collapsed[SECTION_PLACES] {
                break;
            }
            entries.push(SidebarEntry::Place {
                index: self.places.len() + i,
            });
            actions.push(SidebarAction::Go(self.pins[i].clone()));
        }

        entries.push(SidebarEntry::Section {
            label: "Folders".into(),
            collapsed: self.sections_collapsed[SECTION_TREE],
        });
        actions.push(SidebarAction::ToggleSection(SECTION_TREE));
        if !self.sections_collapsed[SECTION_TREE] {
            for (i, row) in self.tree_rows.iter().enumerate() {
                entries.push(SidebarEntry::Tree {
                    index: i,
                    depth: row.depth,
                    expanded: row.expanded,
                });
                actions.push(SidebarAction::Go(row.path.clone()));
            }
        }

        (entries, actions)
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pins.iter().any(|p| crate::pane::paths_equal(p, path))
    }

    /// Pin or unpin a folder. Toggling rather than two commands, because the
    /// menu already knows which state it is in and can say so.
    pub fn toggle_pin(&mut self, path: &str) {
        if let Some(i) = self.pins.iter().position(|p| crate::pane::paths_equal(p, path)) {
            self.pins.remove(i);
        } else if !path.is_empty() {
            self.pins.push(path.to_string());
        }
    }

    /// Push the DIP column widths into the scaled metrics the layout reads.
    /// Called on load, on a DPI change, and after a resize drag.
    pub fn apply_col_widths(&mut self) {
        let s = |v: i32| ((v as f32) * self.metrics.scale).round() as i32;
        self.metrics.col_type_w = s(self.col_widths[0]);
        self.metrics.col_size_w = s(self.col_widths[1]);
        self.metrics.col_date_w = s(self.col_widths[2]);
    }

    /// Which of `col_widths` a sort key names, if any. Name has no entry: it
    /// absorbs whatever the others leave.
    pub fn col_index(key: crate::file_list::SortKey) -> Option<usize> {
        use crate::file_list::SortKey::*;
        match key {
            Type => Some(0),
            Size => Some(1),
            Date => Some(2),
            Name => None,
        }
    }

    /// Keep the sidebar's scroll inside its content.
    pub fn clamp_sidebar_scroll(&mut self) {
        let (entries, _) = self.sidebar_model();
        let content = crate::layout::sidebar_content_h(self.metrics, &entries);
        let view = self.client.h.max(0);
        self.sidebar_scroll = self.sidebar_scroll.clamp(0, (content - view).max(0));
    }

    /// Rebuild the tree's visible rows. Called after anything that changes
    /// what is expanded, not from `layout()`, which runs on every mouse move.
    pub fn refresh_tree(&mut self) {
        let roots: Vec<String> = self.drives.iter().map(|d| d.root.clone()).collect();
        self.tree_rows = self.tree.rows(&roots);
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
                total_lines: self.pane(p).list().total_lines(),
                scroll_offset: self.pane(p).list().scroll_offset,
                grid: self.grid,
            })
            .collect();
        let (entries, _) = self.sidebar_model();
        Layout::compute(
            self.client,
            self.metrics,
            &self.splits,
            self.sidebar_visible,
            &entries,
            self.sidebar_scroll,
            self.inspector,
            &inputs,
            &self.renderer,
        )
    }

    /// Key a cache entry by path and modified time, so an edited file re-renders
    /// rather than showing what it used to look like.
    pub fn image_key(dir: &str, entry: &crate::fs::FileEntry) -> String {
        format!("{}|{}", fs::path_join(dir, &entry.name), entry.modified)
    }

    /// Ask for the images of every cell on screen that has none yet.
    ///
    /// Called from the paint path, like `ensure_preview`: it is the one place
    /// that knows what is actually visible, after every way of changing it.
    /// Bounded per frame so a fast scroll queues a screenful, not a drive.
    pub fn ensure_thumbs(&mut self, hwnd: HWND) {
        if !self.grid {
            return;
        }
        /// Threads started per repaint. A scroll that outruns them simply asks
        /// again next frame for whatever is still missing.
        const PER_FRAME: usize = 16;

        let edge = self.metrics.cell_icon;
        let mut wanted: Vec<(String, Option<String>)> = Vec::new();
        for pid in self.visible().collect::<Vec<_>>() {
            let h = self.list_height(pid);
            let pane = self.pane(pid);
            let dir = pane.current_path().to_string();
            if dir.is_empty() {
                continue;
            }
            let list = pane.list();
            let (start, end) = list.visible_range(h);
            for i in start..end {
                let Some(entry) = list.entries.get(i as usize) else {
                    break;
                };
                let key = Self::image_key(&dir, entry);
                if !self.thumbs.has(&key) && !self.thumbs_pending.contains(&key) {
                    wanted.push((key, entry.extension.clone()));
                }
                if wanted.len() >= PER_FRAME {
                    break;
                }
            }
            if wanted.len() >= PER_FRAME {
                break;
            }
        }

        for (key, extension) in wanted {
            self.thumbs_pending.insert(key.clone());
            let path = key.rsplit_once('|').map(|(p, _)| p.to_string()).unwrap_or_default();
            let owner = ops::OwnerWindow(hwnd);
            // ponytail: a thread per image, at most one per missing visible
            // cell. They are short and the shell serialises behind its own
            // cache anyway; a pool is worth it if a listing of thousands of
            // videos ever makes this visible.
            std::thread::spawn(move || {
                let bmp = crate::preview::cell_image(&path, extension.as_deref(), edge);
                let raw = Box::into_raw(Box::new((key, bmp)));
                let posted = unsafe {
                    PostMessageW(
                        Some(owner.hwnd()),
                        WM_APP_THUMB_READY,
                        WPARAM(0),
                        LPARAM(raw as isize),
                    )
                };
                if posted.is_err() {
                    // The window is gone, so nothing will ever draw this.
                    let (_, bmp) = *unsafe { Box::from_raw(raw) };
                    if let Some(b) = bmp {
                        unsafe {
                            let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                                windows::Win32::Graphics::Gdi::HGDIOBJ(b.0),
                            );
                        }
                    }
                }
            });
        }
    }

    /// Take a finished cell image.
    pub fn finish_thumb(&mut self, key: String, bmp: Option<windows::Win32::Graphics::Gdi::HBITMAP>) {
        self.thumbs_pending.remove(&key);
        self.thumbs.insert(key, bmp);
    }

    /// Push the layout's cells-per-line into every pane's list.
    ///
    /// Only the layout knows it — it depends on the pane's width — and the
    /// list needs it for scrolling and for moving the cursor a line at a time.
    /// Called from the paint path, which is where the final geometry exists,
    /// so a resize, a pane-count change or a hidden sidebar updates it without
    /// each of them having to remember.
    pub fn sync_columns(&mut self) {
        let layout = self.layout();
        for pid in self.visible().collect::<Vec<_>>() {
            let cols = layout.pane(pid).columns_per_line;
            self.pane_mut(pid).set_columns(cols);
        }
    }

    pub fn list_height(&self, pid: PaneId) -> u32 {
        self.layout().pane(pid).list.h.max(0) as u32
    }

    pub fn clamp_split(&mut self) {
        self.splits = Layout::clamp_splits(
            self.client,
            self.metrics,
            self.sidebar_visible,
            self.inspector,
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

    /// What the inspector should be showing: the focused pane's cursor entry,
    /// keyed by path and modified time.
    ///
    /// None when the inspector is hidden or nothing is under the cursor, which
    /// is also how the preview gets dropped when the selection is cleared.
    pub fn preview_key(&self) -> Option<(String, bool, Option<String>)> {
        if !self.inspector {
            return None;
        }
        let pane = self.focused_pane();
        let entry = pane.list().cursor_entry()?;
        let path = fs::path_join(pane.current_path(), &entry.name);
        Some((
            format!("{}|{}", path, entry.modified),
            entry.is_dir,
            entry.extension.clone(),
        ))
    }

    /// Start a preview fetch if what the inspector wants is not what it has.
    ///
    /// Called from the paint path because that is the one place that sees the
    /// selection after every way of changing it — arrows, clicks, a drag band,
    /// type-ahead, a reload — rather than hooking each of them. It spawns a
    /// thread at most; the disk work is the worker's.
    pub fn ensure_preview(&mut self, hwnd: HWND) {
        let Some((key, is_dir, extension)) = self.preview_key() else {
            self.preview = None;
            self.preview_pending = None;
            return;
        };
        if self.preview.as_ref().is_some_and(|(k, _)| *k == key)
            || self.preview_pending.as_deref() == Some(key.as_str())
        {
            return;
        }
        self.preview_pending = Some(key.clone());

        // The key carries the modified time after a '|', which a path cannot
        // contain, so the path is everything before the last one.
        let path = key.rsplit_once('|').map(|(p, _)| p.to_string()).unwrap_or_default();
        let edge = self.metrics.inspector_w.max(64);
        let owner = ops::OwnerWindow(hwnd);
        std::thread::spawn(move || {
            let preview = crate::preview::build(&path, is_dir, extension.as_deref(), edge);
            let raw = Box::into_raw(Box::new((key, preview)));
            let posted = unsafe {
                PostMessageW(
                    Some(owner.hwnd()),
                    WM_APP_PREVIEW_READY,
                    WPARAM(0),
                    LPARAM(raw as isize),
                )
            };
            if posted.is_err() {
                unsafe { drop(Box::from_raw(raw)) };
            }
        });
    }

    /// Record a folder as visited. Most recent first, no duplicates, bounded.
    pub fn remember_visit(&mut self, path: &str) {
        crate::config::push_recent(&mut self.recent, path);
    }

    /// Remember how a folder was sorted, and what a folder was last sorted by.
    ///
    /// Unbounded for the session: one small entry per folder actually sorted by
    /// hand, which is a number of folders a person can produce, not a machine.
    pub fn remember_sort(&mut self, path: &str, key: SortKey, order: SortOrder) {
        if !path.is_empty() {
            self.folder_sort.insert(path.to_lowercase(), (key, order));
        }
    }

    pub fn sort_for(&self, path: &str) -> Option<(SortKey, SortOrder)> {
        self.folder_sort.get(&path.to_lowercase()).copied()
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
            pins: self.pins.clone(),
            recent: self.recent.clone(),
            inspector: self.inspector,
            grid: self.grid,
            // Only what differs from the defaults, so a default that changes
            // in a later version still reaches someone who never rebound it.
            binds: self
                .bindings
                .changed_from(&crate::keys::Bindings::from_defaults(
                    &crate::commands::default_bindings(),
                ))
                .into_iter()
                .map(|(chord, id)| (chord, crate::commands::label_for(id).to_string()))
                .filter(|(_, label)| !label.is_empty())
                .collect(),
            sort: self
                .panes
                .iter()
                .map(|p| {
                    let l = p.list();
                    (sort_key_id(l.sort_key), l.sort_order == SortOrder::Asc)
                })
                .collect(),
            col_widths: self.col_widths,
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
        let frac = self.pane(from).list().scroll_frac;
        for to in self.visible().collect::<Vec<_>>() {
            if to == from {
                continue;
            }
            let h = self.list_height(to);
            let list = self.pane_mut(to).list_mut();
            list.scroll_to(offset, h);
            // Carry the sub-row remainder too, or a synchronised pane would
            // judder against the one being scrolled.
            if list.scroll_offset == offset {
                list.scroll_frac = frac;
            }
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
        let result = fs::list_any(&req.path);
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
    // One guard for every destructive path in the app, drops included: nothing
    // writes inside an archive. Refusing here rather than in each command is
    // what makes that true of paths nobody thought about.
    if op.touches_archive() {
        report_error(
            hwnd,
            "Not supported",
            "Files inside an archive are read-only. Extract them first.",
        );
        return;
    }
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

/// Sort keys are stored in the settings file as small integers. Written out
/// rather than derived, so reordering the enum cannot silently change what a
/// saved file means.
pub fn sort_key_id(key: SortKey) -> u8 {
    match key {
        SortKey::Name => 0,
        SortKey::Type => 1,
        SortKey::Size => 2,
        SortKey::Date => 3,
    }
}

pub fn sort_key_from_id(id: u8) -> SortKey {
    match id {
        1 => SortKey::Type,
        2 => SortKey::Size,
        3 => SortKey::Date,
        _ => SortKey::Name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_key_ids_round_trip() {
        for k in [SortKey::Name, SortKey::Type, SortKey::Size, SortKey::Date] {
            assert_eq!(sort_key_from_id(sort_key_id(k)), k);
        }
    }

    #[test]
    fn an_unknown_sort_id_falls_back_to_name() {
        // A settings file from a future build must not panic an older one.
        assert_eq!(sort_key_from_id(99), SortKey::Name);
    }
}
