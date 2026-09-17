// The single source of truth for where everything is on screen.
//
// This module exists because drawing and hit-testing used to compute geometry
// separately and had already drifted apart: the breadcrumb was painted at a
// fixed 70px stride per crumb while clicks were resolved by dividing the pane
// width evenly, so clicking a crumb navigated to the wrong folder. Anything
// that is drawn is positioned here, and clicks are resolved against the very
// same rectangles, which makes that whole class of bug impossible.
//
// No Win32 types appear here, so all of it is testable without a window.
//
// A pane is five stacked bands:
//
//   +------------------------------+
//   | tab bar        [tabs]     [+]|
//   | toolbar   < > ^  breadcrumb  |
//   | header    Name | Size | Date |
//   | list                      |# |  <- scrollbar gutter on the right
//   | footer    filter...  12 items|
//   +------------------------------+

use crate::file_list::SortKey;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        self.w > 0
            && self.h > 0
            && px >= self.x
            && px < self.right()
            && py >= self.y
            && py < self.bottom()
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    /// The rectangle between two corners, in any order. For a drag band, where
    /// the second corner is wherever the mouse got to.
    pub fn between(a: (i32, i32), b: (i32, i32)) -> Rect {
        Rect::new(
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        )
    }

    /// This rectangle clipped to `other`. Empty when they do not overlap.
    pub fn clamp_to(&self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Rect::new(
            x,
            y,
            (self.right().min(other.right()) - x).max(0),
            (self.bottom().min(other.bottom()) - y).max(0),
        )
    }

    pub fn inset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w - dx * 2, self.h - dy * 2)
    }
    /// Take `n` pixels off the left, returning (taken, remainder).
    pub fn split_left(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.w.max(0));
        (
            Rect::new(self.x, self.y, n, self.h),
            Rect::new(self.x + n, self.y, self.w - n, self.h),
        )
    }
    /// Take `n` pixels off the top, returning (taken, remainder).
    pub fn split_top(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.h.max(0));
        (
            Rect::new(self.x, self.y, self.w, n),
            Rect::new(self.x, self.y + n, self.w, self.h - n),
        )
    }
    /// Take `n` pixels off the bottom, returning (remainder, taken).
    pub fn split_bottom(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.h.max(0));
        (
            Rect::new(self.x, self.y, self.w, self.h - n),
            Rect::new(self.x, self.y + self.h - n, self.w, n),
        )
    }
    /// Take `n` pixels off the right, returning (remainder, taken).
    pub fn split_right(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.w.max(0));
        (
            Rect::new(self.x, self.y, self.w - n, self.h),
            Rect::new(self.x + self.w - n, self.y, n, self.h),
        )
    }
}

/// Anything that can tell us how wide a string will render.
/// Implemented by the renderer with DirectWrite; stubbed in tests.
pub trait TextMeasurer {
    fn measure(&self, text: &str, small: bool) -> f32;
}

/// Every size in the UI, already scaled for the monitor's DPI.
///
/// Holding these in one place is what makes the app DPI-correct: the layout is
/// computed in physical pixels for the current monitor, so nothing is ever
/// bitmap-stretched the way it was when the process was DPI-unaware.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub scale: f32,

    pub tab_bar_h: i32,
    pub tab_min_w: i32,
    pub tab_max_w: i32,
    pub tab_close_w: i32,
    pub new_tab_w: i32,

    pub toolbar_h: i32,
    pub nav_btn_w: i32,
    pub header_h: i32,
    pub footer_h: i32,
    pub row_h: i32,

    pub sidebar_w: i32,
    /// The inspector panel on the right. Wide enough for a readable preview
    /// without taking a listing's worth of window.
    pub inspector_w: i32,
    pub sidebar_section_h: i32,
    pub sidebar_row_h: i32,
    /// A drive row is taller than a place row: it carries a capacity bar.
    pub sidebar_drive_h: i32,
    pub capacity_bar_h: i32,

    pub divider_w: i32,
    pub scrollbar_w: i32,
    pub scrollbar_min_thumb: i32,
    pub min_pane_w: i32,
    pub icon_size: i32,
    /// One cell of the icon view, and the icon drawn inside it. Wide enough
    /// for a readable two-line name under a thumbnail.
    pub cell_w: i32,
    pub cell_h: i32,
    pub cell_icon: i32,
    pub pad: i32,
    pub radius: f32,

    pub col_type_w: i32,
    pub col_size_w: i32,
    pub col_date_w: i32,
    pub col_min_name_w: i32,
    pub crumb_sep_w: i32,
    pub filter_max_w: i32,
}

impl Metrics {
    pub fn for_dpi(dpi: u32) -> Self {
        let scale = dpi as f32 / 96.0;
        let s = |v: f32| (v * scale).round() as i32;
        Self {
            scale,

            tab_bar_h: s(30.0),
            tab_min_w: s(96.0),
            tab_max_w: s(190.0),
            tab_close_w: s(18.0),
            new_tab_w: s(26.0),

            toolbar_h: s(30.0),
            nav_btn_w: s(26.0),
            header_h: s(24.0),
            footer_h: s(24.0),
            // Dense by design: this is a list you scan, not a form you read.
            row_h: s(22.0),

            sidebar_w: s(196.0),
            inspector_w: s(300.0),
            sidebar_section_h: s(26.0),
            sidebar_row_h: s(24.0),
            sidebar_drive_h: s(46.0),
            capacity_bar_h: s(3.0),

            divider_w: s(6.0),
            scrollbar_w: s(11.0),
            scrollbar_min_thumb: s(28.0),
            min_pane_w: s(240.0),
            icon_size: s(16.0),
            cell_w: s(118.0),
            cell_h: s(124.0),
            cell_icon: s(72.0),
            pad: s(8.0),
            radius: 4.0 * scale,

            col_type_w: s(96.0),
            col_size_w: s(84.0),
            col_date_w: s(124.0),
            col_min_name_w: s(120.0),
            crumb_sep_w: s(13.0),
            filter_max_w: s(220.0),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::for_dpi(96)
    }
}

/// Which pane, by position from the left. The window holds up to `MAX_PANES`
/// of them; `pane_count` says how many are on screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PaneId(pub usize);

/// More than four panes on one screen leaves each too narrow to read, and the
/// minimum-width clamp would collapse them to even thirds anyway.
pub const MAX_PANES: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavButton {
    Back,
    Forward,
    Up,
}

#[derive(Clone, Copy, Debug)]
pub struct TabRect {
    pub full: Rect,
    /// The little x. Empty when the tab is too narrow to show one.
    pub close: Rect,
}

#[derive(Clone, Copy, Debug)]
pub struct CrumbRect {
    pub rect: Rect,
    /// Index into the original path segments. Not the same as position in the
    /// crumb list once leading crumbs have been elided.
    pub segment_index: usize,
    /// True for the synthesised ellipsis crumb standing in for hidden parents.
    pub is_ellipsis: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ColumnRect {
    pub key: SortKey,
    pub rect: Rect,
    /// The strip on this column's left edge that resizes it. Empty for Name,
    /// which has no edge of its own: it absorbs whatever the others leave.
    /// A rect rather than a constant so the grab area scales with the monitor.
    pub edge: Rect,
}

/// What a sidebar row represents. The caller builds the list; the layout only
/// assigns rectangles, so adding a new kind of shortcut needs no change here.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarEntry {
    Section { label: String, collapsed: bool },
    /// Index into the caller's drive list. `used` in 0..=1 draws the capacity bar.
    Drive { index: usize, used: Option<f32> },
    /// Index into the caller's shortcut list.
    Place { index: usize },
    /// A folder in the tree. `index` is into the caller's row list; `depth`
    /// drives the indent, and every folder gets a chevron because finding out
    /// whether it has children costs a directory read.
    Tree { index: usize, depth: usize, expanded: bool },
}

#[derive(Clone, Debug)]
pub struct SidebarRow {
    pub rect: Rect,
    /// Thin capacity bar under a drive's label. Empty for everything else.
    pub capacity: Rect,
    /// Chevron target for a section header. Empty for everything else.
    pub chevron: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct PaneLayout {
    pub bounds: Rect,
    pub tab_bar: Rect,
    pub tabs: Vec<TabRect>,
    pub new_tab: Rect,

    pub toolbar: Rect,
    pub nav_back: Rect,
    pub nav_forward: Rect,
    pub nav_up: Rect,
    pub breadcrumb: Rect,
    pub crumbs: Vec<CrumbRect>,

    pub header: Rect,
    pub columns: Vec<ColumnRect>,

    /// The rows area, excluding the scrollbar gutter.
    pub list: Rect,
    pub scrollbar: Rect,
    pub thumb: Rect,
    /// Rows fully visible in `list`.
    /// Whole lines that fit in the list area.
    pub visible_rows: u32,
    /// Entries per line: 1 in the details view, as many as fit in the icon
    /// view. The one number that makes a grid out of a list — every index is
    /// still flat, only the mapping from index to rectangle changes.
    pub columns_per_line: u32,
    /// Height of one line: a row in the details view, a cell in the icon view.
    pub line_h: i32,

    pub footer: Rect,
    pub filter: Rect,
    pub counts: Rect,
}

impl PaneLayout {
    /// Absolute row index under a point, if the point is over a real row.
    /// The rectangle one entry occupies, whichever view is showing.
    ///
    /// Indices stay flat — `columns_per_line` and `line_h` are the only things
    /// that differ between a details row and an icon cell — so everything that
    /// works in indices (selection, sorting, the cursor) needs no view branch.
    /// `None` only when the entry is entirely outside the rows area.
    pub fn cell(&self, index: u32, scroll_px: i32) -> Option<Rect> {
        let cols = self.columns_per_line.max(1);
        if self.line_h <= 0 || self.list.w <= 0 {
            return None;
        }
        let line = (index / cols) as i32;
        let col = (index % cols) as i32;
        let y = self.list.y + line * self.line_h - scroll_px;
        if y + self.line_h <= self.list.y || y >= self.list.bottom() {
            return None;
        }
        let w = if cols == 1 {
            self.list.w
        } else {
            self.list.w / cols as i32
        };
        Some(Rect::new(self.list.x + col * w, y, w, self.line_h))
    }

    /// The icon and name-text rectangles inside one entry's cell.
    ///
    /// One formula, used by the painter and by the inline rename editor, so the
    /// box you type in sits exactly where the name was drawn. A row half off
    /// the bottom edge still has to be painted and the clip takes care of the
    /// rest; a caller that needs the cell *whole* — the rename editor does, a
    /// box hanging out of its panel is not usable — checks what it gets back.
    pub fn cell_parts(&self, index: u32, scroll_px: i32, m: Metrics) -> Option<(Rect, Rect)> {
        let cell = self.cell(index, scroll_px)?;
        if self.columns_per_line > 1 {
            // Icon view: a big icon over a two-line name, both centred.
            let icon = Rect::new(
                cell.x + (cell.w - m.cell_icon) / 2,
                cell.y + m.pad,
                m.cell_icon,
                m.cell_icon,
            );
            let label = Rect::new(
                cell.x + m.pad / 2,
                icon.bottom() + m.pad / 2,
                (cell.w - m.pad).max(0),
                (cell.bottom() - icon.bottom() - m.pad).max(0),
            );
            return Some((icon, label));
        }
        // Details: a small icon in the Name column, the name after it.
        let nc = self
            .columns
            .iter()
            .find(|c| c.key == SortKey::Name)
            .map(|c| c.rect)?;
        let icon = Rect::new(
            nc.x + m.pad,
            cell.y + (self.line_h - m.icon_size) / 2,
            m.icon_size,
            m.icon_size,
        );
        let name = Rect::new(
            icon.right() + m.pad,
            cell.y,
            (nc.right() - icon.right() - m.pad * 2).max(0),
            self.line_h,
        );
        Some((icon, name))
    }

    /// Entries a selection band covers.
    ///
    /// Each candidate cell is tested against the band rather than a row range
    /// being derived from it, because in the icon view a band is a rectangle
    /// over a grid and only some of a line is inside it. In the details view
    /// cells span the width, so this comes out as the row range it used to be.
    pub fn band_indices(&self, band: Rect, scroll_px: i32, total: u32) -> Vec<u32> {
        let cols = self.columns_per_line.max(1);
        if self.line_h <= 0 || total == 0 {
            return Vec::new();
        }
        // A drag straight down or straight across has no width or no height,
        // and a zero-area rectangle intersects nothing. One pixel is what the
        // gesture means.
        let band = Rect::new(band.x, band.y, band.w.max(1), band.h.max(1));
        // Only lines on screen can be covered: the band is clamped to the list.
        let band = band.clamp_to(self.list);
        if band.is_empty() {
            return Vec::new();
        }
        let first_line = ((band.y - self.list.y + scroll_px) / self.line_h).max(0);
        let last_line = (band.bottom() - 1 - self.list.y + scroll_px) / self.line_h;
        let mut out = Vec::new();
        for line in first_line..=last_line.max(first_line) {
            for col in 0..cols {
                let index = line as u32 * cols + col;
                if index >= total {
                    return out;
                }
                if self
                    .cell(index, scroll_px)
                    .is_some_and(|r| !r.clamp_to(band).is_empty())
                {
                    out.push(index);
                }
            }
        }
        out
    }

    /// The entry under a point, or None if there is none there.
    pub fn cell_at(&self, px: i32, py: i32, scroll_px: i32, total: u32) -> Option<u32> {
        let cols = self.columns_per_line.max(1);
        if self.line_h <= 0 || !self.list.contains(px, py) {
            return None;
        }
        let line = ((py - self.list.y + scroll_px) / self.line_h) as i64;
        let w = if cols == 1 {
            self.list.w
        } else {
            self.list.w / cols as i32
        };
        if w <= 0 {
            return None;
        }
        // A window wider than cols*cell_w leaves a strip on the right that
        // belongs to no cell; clamping there would select the last column from
        // empty space.
        let col = ((px - self.list.x) / w) as i64;
        if col < 0 || col >= cols as i64 {
            return None;
        }
        let index = line * cols as i64 + col;
        if index >= 0 && (index as u64) < total as u64 {
            Some(index as u32)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub metrics: Metrics,
    pub client: Rect,
    pub sidebar: Rect,
    pub sidebar_rows: Vec<SidebarRow>,
    /// The inspector panel, or empty when it is hidden. Mirrors the sidebar:
    /// taken off the body before the panes are laid out, so nothing else has
    /// to know it exists.
    pub inspector: Rect,
    pub panes: Vec<PaneLayout>,
    /// One fewer than `panes`. Empty when a single pane fills the window.
    pub dividers: Vec<Rect>,
    /// Returned for a pane id that is not on screen. Every rect in it is empty,
    /// so drawing it paints nothing and hit-testing it matches nothing — which
    /// is why no caller needs a "is this pane visible" branch.
    fallback: PaneLayout,
}

/// What is under the cursor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Nothing,
    /// Index into `Layout::sidebar_rows`.
    Sidebar(usize),
    SidebarChevron(usize),
    /// Index of the divider, counting from the left.
    Divider(usize),
    Tab(PaneId, usize),
    TabClose(PaneId, usize),
    NewTab(PaneId),
    Nav(PaneId, NavButton),
    Crumb(PaneId, usize),
    Column(PaneId, SortKey),
    /// The grab strip on a column's left edge. Only the optional columns have
    /// one: Name absorbs whatever is left, so there is nothing to drag on it.
    ColumnEdge(PaneId, SortKey),
    /// Absolute row index within the pane's list.
    Row(PaneId, u32),
    ListBackground(PaneId),
    ScrollbarThumb(PaneId),
    /// Click on the track above or below the thumb; the f32 is 0..1 along the track.
    ScrollbarTrack(PaneId, f32),
    Filter(PaneId),
}

/// Everything the layout needs to know about one pane to size it.
pub struct PaneInput<'a> {
    pub tab_labels: &'a [String],
    pub crumbs: &'a [String],
    /// Lines of cells, not entries: in the icon view one line holds several.
    /// It is what the scrollbar measures against.
    pub total_lines: u32,
    pub scroll_offset: u32,
    /// Icon view rather than the details list.
    pub grid: bool,
}

impl Layout {
    /// Compute the whole frame.
    ///
    /// `splits` holds one fraction of the body width per divider, already
    /// clamped by `clamp_splits`. Fractions rather than pixels, so resizing the
    /// window keeps the panes in proportion instead of squeezing the last one.
    pub fn compute(
        client: Rect,
        metrics: Metrics,
        splits: &[f32],
        sidebar_visible: bool,
        sidebar_entries: &[SidebarEntry],
        // How far the sidebar is scrolled, in pixels.
        sidebar_scroll: i32,
        inspector_visible: bool,
        panes: &[PaneInput],
        measurer: &dyn TextMeasurer,
    ) -> Layout {
        let m = metrics;
        let body = client;

        let (sidebar, body) = if sidebar_visible {
            body.split_left(m.sidebar_w)
        } else {
            (Rect::default(), body)
        };
        let (body, inspector) = if inspector_visible {
            body.split_right(m.inspector_w)
        } else {
            (body, Rect::default())
        };

        let sidebar_rows = sidebar_rows(sidebar, m, sidebar_entries, sidebar_scroll);
        let (bounds, dividers) = split_body(body, m, splits, panes.len());

        Layout {
            metrics: m,
            client,
            sidebar,
            sidebar_rows,
            inspector,
            panes: bounds
                .iter()
                .zip(panes)
                .map(|(b, input)| pane_layout(*b, m, input, measurer))
                .collect(),
            dividers,
            fallback: PaneLayout::default(),
        }
    }

    /// Clamp proposed divider fractions so every pane keeps a usable width.
    /// A count that does not match the fractions, or a window too narrow to
    /// honour the minimums, falls back to equal shares.
    pub fn clamp_splits(
        client: Rect,
        m: Metrics,
        sidebar_visible: bool,
        inspector_visible: bool,
        count: usize,
        splits: &[f32],
    ) -> Vec<f32> {
        if count < 2 {
            return Vec::new();
        }
        let even = || (1..count).map(|i| i as f32 / count as f32).collect::<Vec<f32>>();
        let body_x = client.x + if sidebar_visible { m.sidebar_w } else { 0 };
        let body_w =
            client.right() - body_x - if inspector_visible { m.inspector_w } else { 0 };
        let needed = m.min_pane_w * count as i32 + m.divider_w * (count as i32 - 1);
        if splits.len() != count - 1 || body_w < needed {
            return even();
        }

        let mut px: Vec<i32> = splits
            .iter()
            .map(|f| (f * body_w as f32).round() as i32)
            .collect();
        // Left to right pushes each divider past its predecessor's minimum,
        // then right to left does the same from the far edge. Both passes are
        // needed: one alone can only protect one end.
        for i in 0..px.len() {
            let low = if i == 0 {
                m.min_pane_w
            } else {
                px[i - 1] + m.divider_w + m.min_pane_w
            };
            px[i] = px[i].max(low);
        }
        for i in (0..px.len()).rev() {
            let high = if i + 1 == px.len() {
                body_w - m.min_pane_w - m.divider_w
            } else {
                px[i + 1] - m.divider_w - m.min_pane_w
            };
            px[i] = px[i].min(high);
        }
        px.iter().map(|p| *p as f32 / body_w as f32).collect()
    }

    /// The fraction that puts divider `index` at client x `x`.
    pub fn split_fraction_at(&self, x: i32) -> f32 {
        let body_x = self.sidebar.right().max(self.client.x);
        let body_w = (self.client.right() - body_x).max(1);
        ((x - body_x) as f32 / body_w as f32).clamp(0.0, 1.0)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        for (i, d) in self.dividers.iter().enumerate() {
            if d.contains(x, y) {
                return Hit::Divider(i);
            }
        }
        if self.sidebar.contains(x, y) {
            for (i, r) in self.sidebar_rows.iter().enumerate() {
                if r.chevron.contains(x, y) {
                    return Hit::SidebarChevron(i);
                }
                if r.rect.contains(x, y) {
                    return Hit::Sidebar(i);
                }
            }
            return Hit::Nothing;
        }
        for (i, p) in self.panes.iter().enumerate() {
            if let Some(h) = hit_pane(p, PaneId(i), x, y) {
                return h;
            }
        }
        Hit::Nothing
    }

    /// The layout of one pane. A pane that is not on screen gets an all-empty
    /// layout rather than a panic, so callers holding a stale id stay harmless.
    pub fn pane(&self, pid: PaneId) -> &PaneLayout {
        self.panes.get(pid.0).unwrap_or(&self.fallback)
    }

    /// The pane whose bounds contain a point, if any.
    pub fn pane_at(&self, x: i32, y: i32) -> Option<PaneId> {
        self.panes
            .iter()
            .position(|p| p.bounds.contains(x, y))
            .map(PaneId)
    }
}

/// Carve the body into `count` pane rectangles with a divider between each.
fn split_body(body: Rect, m: Metrics, splits: &[f32], count: usize) -> (Vec<Rect>, Vec<Rect>) {
    if count == 0 {
        return (Vec::new(), Vec::new());
    }
    if count == 1 {
        return (vec![body], Vec::new());
    }
    let mut dividers = Vec::with_capacity(count - 1);
    for f in splits.iter().take(count - 1) {
        let x = body.x + (f * body.w as f32).round() as i32;
        dividers.push(Rect::new(x.clamp(body.x, body.right()), body.y, m.divider_w, body.h));
    }
    let mut bounds = Vec::with_capacity(count);
    let mut left = body.x;
    for d in &dividers {
        bounds.push(Rect::new(left, body.y, (d.x - left).max(0), body.h));
        left = d.right();
    }
    bounds.push(Rect::new(left, body.y, (body.right() - left).max(0), body.h));
    (bounds, dividers)
}

fn hit_pane(p: &PaneLayout, pid: PaneId, x: i32, y: i32) -> Option<Hit> {
    if !p.bounds.contains(x, y) {
        return None;
    }
    if p.tab_bar.contains(x, y) {
        if p.new_tab.contains(x, y) {
            return Some(Hit::NewTab(pid));
        }
        for (i, t) in p.tabs.iter().enumerate() {
            if t.close.contains(x, y) {
                return Some(Hit::TabClose(pid, i));
            }
            if t.full.contains(x, y) {
                return Some(Hit::Tab(pid, i));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.toolbar.contains(x, y) {
        if p.nav_back.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Back));
        }
        if p.nav_forward.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Forward));
        }
        if p.nav_up.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Up));
        }
        for c in &p.crumbs {
            if c.rect.contains(x, y) {
                return Some(Hit::Crumb(pid, c.segment_index));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.header.contains(x, y) {
        // The edge wins over the column it belongs to, or a column could never
        // be resized: its header button covers the same pixels.
        for c in &p.columns {
            if c.edge.contains(x, y) {
                return Some(Hit::ColumnEdge(pid, c.key));
            }
        }
        for c in &p.columns {
            if c.rect.contains(x, y) {
                return Some(Hit::Column(pid, c.key));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.footer.contains(x, y) {
        if p.filter.contains(x, y) {
            return Some(Hit::Filter(pid));
        }
        return Some(Hit::Nothing);
    }
    if p.scrollbar.contains(x, y) {
        if p.thumb.contains(x, y) {
            return Some(Hit::ScrollbarThumb(pid));
        }
        let t = if p.scrollbar.h > 0 {
            ((y - p.scrollbar.y) as f32 / p.scrollbar.h as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return Some(Hit::ScrollbarTrack(pid, t));
    }
    if p.list.contains(x, y) {
        return Some(Hit::ListBackground(pid));
    }
    Some(Hit::Nothing)
}

/// Distance from a sidebar row's left edge to where its text starts.
/// The renderer places the icon using the same arithmetic, so the capacity bar
/// always sits flush under the label rather than under the icon.
pub fn sidebar_text_offset(m: Metrics) -> i32 {
    m.pad + m.pad / 2 + m.icon_size + m.pad
}

/// How far in a tree row at `depth` sits. Shared by the layout and the
/// renderer so the chevron, the icon and the label line up by construction.
pub fn tree_indent(m: Metrics, depth: usize) -> i32 {
    // Capped: past a dozen levels an indent per level leaves no room for names.
    (depth.min(12) as i32) * (m.icon_size - m.pad / 4).max(1)
}

/// Total height every sidebar entry would need, unscrolled. The caller clamps
/// its scroll against this.
pub fn sidebar_content_h(m: Metrics, entries: &[SidebarEntry]) -> i32 {
    m.pad + entries.iter().map(|e| sidebar_row_h(m, e)).sum::<i32>()
}

fn sidebar_row_h(m: Metrics, e: &SidebarEntry) -> i32 {
    match e {
        SidebarEntry::Section { .. } => m.sidebar_section_h,
        SidebarEntry::Drive { used: Some(_), .. } => m.sidebar_drive_h,
        SidebarEntry::Drive { .. } => m.sidebar_row_h,
        SidebarEntry::Place { .. } => m.sidebar_row_h,
        SidebarEntry::Tree { .. } => m.sidebar_row_h,
    }
}

fn sidebar_rows(
    sidebar: Rect,
    m: Metrics,
    entries: &[SidebarEntry],
    scroll: i32,
) -> Vec<SidebarRow> {
    if sidebar.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(entries.len());
    let mut y = sidebar.y + m.pad / 2 - scroll;
    for e in entries {
        let h = sidebar_row_h(m, e);
        let rect = Rect::new(sidebar.x, y, sidebar.w, h);
        // Rows off either end get an empty rect so indices still line up with
        // the caller's list; an empty rect can never be hit or drawn.
        let rect = if rect.bottom() > sidebar.bottom() || rect.y < sidebar.y {
            Rect::default()
        } else {
            rect
        };

        let capacity = match e {
            SidebarEntry::Drive { used: Some(_), .. } if !rect.is_empty() => {
                let text_x = rect.x + sidebar_text_offset(m);
                Rect::new(
                    text_x,
                    rect.bottom() - m.pad / 2 - m.capacity_bar_h,
                    (rect.right() - text_x - m.pad).max(0),
                    m.capacity_bar_h,
                )
            }
            _ => Rect::default(),
        };
        let chevron = match e {
            SidebarEntry::Section { .. } if !rect.is_empty() => Rect::new(
                rect.right() - m.pad - m.icon_size,
                rect.y,
                m.icon_size,
                rect.h,
            ),
            // A tree row's chevron leads its label rather than trailing the
            // row, so a click at the indent opens the folder and a click on
            // the name goes there.
            SidebarEntry::Tree { depth, .. } if !rect.is_empty() => Rect::new(
                rect.x + m.pad / 2 + tree_indent(m, *depth),
                rect.y,
                m.icon_size,
                rect.h,
            ),
            _ => Rect::default(),
        };

        out.push(SidebarRow {
            rect,
            capacity,
            chevron,
        });
        y += h;
    }
    out
}

fn pane_layout(
    bounds: Rect,
    m: Metrics,
    input: &PaneInput,
    measurer: &dyn TextMeasurer,
) -> PaneLayout {
    if bounds.w <= 0 || bounds.h <= 0 {
        return PaneLayout {
            bounds,
            ..Default::default()
        };
    }

    let (tab_bar, rest) = bounds.split_top(m.tab_bar_h);
    let (toolbar, rest) = rest.split_top(m.toolbar_h);
    // The icon view has no columns, so it has no column header to click.
    let (header, rest) = rest.split_top(if input.grid { 0 } else { m.header_h });
    let (list_area, footer) = rest.split_bottom(m.footer_h);
    let (list, scrollbar) = list_area.split_right(m.scrollbar_w);

    let tabs = tab_rects(tab_bar, m, input.tab_labels, measurer);
    let new_tab = {
        let after = tabs.last().map(|t| t.full.right()).unwrap_or(tab_bar.x);
        let x = after.min(tab_bar.right() - m.new_tab_w);
        if x >= tab_bar.x && tab_bar.w >= m.new_tab_w {
            Rect::new(x, tab_bar.y, m.new_tab_w, tab_bar.h)
        } else {
            Rect::default()
        }
    };

    // Toolbar: three nav buttons, then the breadcrumb fills what is left.
    let (nav_back, r) = toolbar.split_left(m.nav_btn_w);
    let (nav_forward, r) = r.split_left(m.nav_btn_w);
    let (nav_up, breadcrumb) = r.split_left(m.nav_btn_w);
    let crumbs = crumb_rects(breadcrumb, m, input.crumbs, measurer);

    let columns = column_rects(header, m);

    // Details is the grid with one column: everything below works off these
    // two numbers, so neither the painter nor hit-testing needs a view branch.
    let (columns_per_line, line_h) = if input.grid {
        ((list.w / m.cell_w).max(1) as u32, m.cell_h)
    } else {
        (1, m.row_h)
    };
    let visible_rows = if line_h > 0 {
        (list.h / line_h).max(0) as u32
    } else {
        0
    };
    let thumb = thumb_rect(
        scrollbar,
        m,
        input.total_lines,
        visible_rows,
        input.scroll_offset,
    );

    // Footer: filter field on the left, counts right-aligned.
    let filter_w = m.filter_max_w.min((footer.w / 2).max(0));
    let (filter, counts) = footer.split_left(filter_w);

    PaneLayout {
        bounds,
        tab_bar,
        tabs,
        new_tab,
        toolbar,
        nav_back,
        nav_forward,
        nav_up,
        breadcrumb,
        crumbs,
        header,
        columns,
        list,
        scrollbar,
        thumb,
        visible_rows,
        columns_per_line,
        line_h,
        footer,
        filter: filter.inset(m.pad / 2, m.pad / 4),
        counts,
    }
}

fn tab_rects(bar: Rect, m: Metrics, labels: &[String], measurer: &dyn TextMeasurer) -> Vec<TabRect> {
    if labels.is_empty() {
        return Vec::new();
    }
    // Width the tabs would like, based on their actual text.
    let natural: Vec<i32> = labels
        .iter()
        .map(|l| {
            let text = measurer.measure(l, true).ceil() as i32;
            // icon + text + close
            (text + m.icon_size + m.pad * 3 + m.tab_close_w).clamp(m.tab_min_w, m.tab_max_w)
        })
        .collect();

    let available = (bar.w - m.new_tab_w).max(0);
    let total: i32 = natural.iter().sum();
    // Shrink proportionally when they will not all fit, with a hard floor so a
    // tab never collapses to nothing.
    let widths: Vec<i32> = if total <= available || total == 0 {
        natural
    } else {
        let floor = m.tab_min_w.min(available / labels.len().max(1) as i32).max(1);
        natural
            .iter()
            .map(|w| ((*w as i64 * available as i64) / total as i64).max(floor as i64) as i32)
            .collect()
    };

    let mut out = Vec::with_capacity(labels.len());
    let mut x = bar.x;
    for w in widths {
        let full = Rect::new(x, bar.y, w.min((bar.right() - x).max(0)), bar.h);
        let close = if full.w > m.tab_close_w * 3 {
            Rect::new(
                full.right() - m.tab_close_w - m.pad / 2,
                full.y + (full.h - m.tab_close_w) / 2,
                m.tab_close_w,
                m.tab_close_w,
            )
        } else {
            Rect::default()
        };
        out.push(TabRect { full, close });
        x += w;
        if x >= bar.right() {
            break;
        }
    }
    out
}

fn crumb_rects(
    bar: Rect,
    m: Metrics,
    segments: &[String],
    measurer: &dyn TextMeasurer,
) -> Vec<CrumbRect> {
    if segments.is_empty() || bar.w <= 0 {
        return Vec::new();
    }
    let widths: Vec<i32> = segments
        .iter()
        .map(|s| measurer.measure(s, true).ceil() as i32 + m.pad)
        .collect();

    let available = bar.w - m.pad * 2;
    let sep = m.crumb_sep_w;

    // Keep as many trailing crumbs as fit; the current folder always survives.
    let mut first = 0usize;
    loop {
        let ellipsis = if first > 0 { m.crumb_sep_w + sep } else { 0 };
        let total: i32 = widths[first..].iter().sum::<i32>()
            + sep * (segments.len() - first).saturating_sub(1) as i32
            + ellipsis;
        if total <= available || first + 1 >= segments.len() {
            break;
        }
        first += 1;
    }

    let mut out = Vec::new();
    let mut x = bar.x + m.pad;

    if first > 0 {
        // One crumb standing in for the hidden parents. Clicking it goes to the
        // deepest hidden ancestor, which is what the user is reaching for.
        out.push(CrumbRect {
            rect: Rect::new(x, bar.y, m.crumb_sep_w, bar.h),
            segment_index: first - 1,
            is_ellipsis: true,
        });
        x += m.crumb_sep_w + sep;
    }

    for i in first..segments.len() {
        let w = widths[i].min((bar.right() - x).max(0));
        if w <= 0 {
            break;
        }
        out.push(CrumbRect {
            rect: Rect::new(x, bar.y, w, bar.h),
            segment_index: i,
            is_ellipsis: false,
        });
        x += w;
        if i + 1 < segments.len() {
            x += sep;
        }
    }
    out
}

fn column_rects(header: Rect, m: Metrics) -> Vec<ColumnRect> {
    if header.w <= 0 {
        return Vec::new();
    }
    // Every column but Name has a fixed width and Name absorbs the rest. When
    // the pane is too narrow the optional ones drop off, last-listed first,
    // rather than squeezing Name to nothing.
    //
    // The order here is the order they are given up in: Type goes before Date,
    // which goes before Size. Size is the one worth keeping longest, because a
    // file manager without it is a list of names.
    let optional = [
        (SortKey::Size, m.col_size_w),
        (SortKey::Date, m.col_date_w),
        (SortKey::Type, m.col_type_w),
    ];
    let mut name_w = header.w;
    let mut kept = Vec::new();
    for (key, w) in optional {
        if name_w - w < m.col_min_name_w {
            break;
        }
        name_w -= w;
        kept.push((key, w));
    }

    // Laid out left to right in reading order, whatever the drop-off order.
    let display = [SortKey::Type, SortKey::Size, SortKey::Date];
    let mut cols = vec![ColumnRect {
        key: SortKey::Name,
        rect: Rect::new(header.x, header.y, name_w, header.h),
        edge: Rect::default(),
    }];
    let mut x = header.x + name_w;
    for key in display {
        if let Some((_, w)) = kept.iter().find(|(k, _)| *k == key) {
            cols.push(ColumnRect {
                key,
                rect: Rect::new(x, header.y, *w, header.h),
                edge: Rect::new(x - m.pad / 2, header.y, m.pad, header.h),
            });
            x += w;
        }
    }
    cols
}

fn thumb_rect(track: Rect, m: Metrics, total: u32, visible: u32, offset: u32) -> Rect {
    if track.h <= 0 || total == 0 || visible >= total {
        return Rect::default();
    }
    let frac = visible as f32 / total as f32;
    let h = ((track.h as f32 * frac) as i32)
        .max(m.scrollbar_min_thumb)
        .min(track.h);
    let max_off = total.saturating_sub(visible).max(1);
    let t = (offset as f32 / max_off as f32).clamp(0.0, 1.0);
    let y = track.y + ((track.h - h) as f32 * t) as i32;
    Rect::new(track.x, y, track.w, h)
}

/// Invert `thumb_rect`: where should scroll_offset land for a drag to `py`?
pub fn scroll_offset_for_thumb_y(track: Rect, m: Metrics, total: u32, visible: u32, py: i32) -> u32 {
    if total == 0 || visible >= total || track.h <= 0 {
        return 0;
    }
    let frac = visible as f32 / total as f32;
    let h = ((track.h as f32 * frac) as i32)
        .max(m.scrollbar_min_thumb)
        .min(track.h);
    let span = (track.h - h).max(1);
    let t = ((py - track.y) as f32 / span as f32).clamp(0.0, 1.0);
    let max_off = total.saturating_sub(visible);
    (t * max_off as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10px wide at 96dpi. Enough to reason about layout.
    struct FixedWidth;
    impl TextMeasurer for FixedWidth {
        fn measure(&self, text: &str, _small: bool) -> f32 {
            text.chars().count() as f32 * 10.0
        }
    }

    fn input<'a>(tabs: &'a [String], crumbs: &'a [String], rows: u32, scroll: u32) -> PaneInput<'a> {
        PaneInput {
            tab_labels: tabs,
            crumbs,
            total_lines: rows,
            grid: false,
            scroll_offset: scroll,
        }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn sidebar_of(n_drives: usize) -> Vec<SidebarEntry> {
        let mut v = vec![SidebarEntry::Section {
            label: "Drives".into(),
            collapsed: false,
        }];
        for i in 0..n_drives {
            v.push(SidebarEntry::Drive {
                index: i,
                used: Some(0.5),
            });
        }
        v.push(SidebarEntry::Section {
            label: "Places".into(),
            collapsed: false,
        });
        v.push(SidebarEntry::Place { index: 0 });
        v
    }

    /// `n` panes sharing the body evenly.
    fn build_n(n: usize, w: i32, h: i32, crumbs: &[String]) -> Layout {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, w, h);
        let tabs = strs(&["Foo"]);
        let splits = Layout::clamp_splits(client, m, true, false, n, &[]);
        let inputs: Vec<PaneInput> = (0..n)
            .map(|_| input(&tabs, crumbs, 100, 0))
            .collect();
        Layout::compute(client, m, &splits, true, &sidebar_of(2), 0, false, &inputs, &FixedWidth)
    }

    fn build(w: i32, h: i32, crumbs: &[String]) -> Layout {
        build_n(2, w, h, crumbs)
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10, 10, 5, 5);
        assert!(r.contains(10, 10));
        assert!(r.contains(14, 14));
        assert!(!r.contains(15, 15));
        assert!(!r.contains(9, 10));
    }

    #[test]
    fn empty_rect_contains_nothing() {
        assert!(!Rect::new(0, 0, 0, 10).contains(0, 0));
    }

    #[test]
    fn panes_do_not_overlap_and_fill_the_body() {
        for n in 1..=MAX_PANES {
            let l = build_n(n, 1800, 800, &strs(&["C:\\", "Users"]));
            assert_eq!(l.panes.len(), n);
            assert_eq!(l.dividers.len(), n - 1);
            assert_eq!(l.panes[0].bounds.x, l.sidebar.right(), "n={}", n);
            for i in 0..n - 1 {
                assert_eq!(l.panes[i].bounds.right(), l.dividers[i].x, "n={}", n);
                assert_eq!(l.dividers[i].right(), l.panes[i + 1].bounds.x, "n={}", n);
            }
            assert_eq!(l.panes[n - 1].bounds.right(), l.client.right(), "n={}", n);
        }
    }

    #[test]
    fn every_pane_keeps_its_minimum_width() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1800, 800);
        // Three dividers crowded into the left edge: the clamp has to push them
        // apart, not let two panes collapse to nothing.
        let splits = Layout::clamp_splits(client, m, true, false, 4, &[0.01, 0.02, 0.03]);
        let inputs: Vec<PaneInput> = (0..4).map(|_| input(&[], &[], 0, 0)).collect();
        let l = Layout::compute(client, m, &splits, true, &[], 0, false, &inputs, &FixedWidth);
        for p in &l.panes {
            assert!(p.bounds.w >= m.min_pane_w, "pane too narrow: {:?}", p.bounds);
        }
    }

    #[test]
    fn a_window_too_narrow_for_the_minimums_shares_evenly() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 400, 600);
        let splits = Layout::clamp_splits(client, m, false, false, 4, &[0.9, 0.92, 0.95]);
        assert_eq!(splits, vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn a_dropped_pane_leaves_no_divider_behind() {
        // Ctrl+1 after Ctrl+4: the count changes, the stale fractions do not.
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1800, 800);
        assert!(Layout::clamp_splits(client, m, true, false, 1, &[0.25, 0.5, 0.75]).is_empty());
    }

    #[test]
    fn single_pane_fills_the_body_and_has_no_divider() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let l = Layout::compute(
            client,
            m,
            &[],
            true,
            &sidebar_of(2),
            0,
            false,
            &[input(&tabs, &crumbs, 10, 0)],
            &FixedWidth,
        );
        assert!(l.dividers.is_empty());
        // A pane that is not on screen still answers, with nothing in it.
        assert!(l.pane(PaneId(1)).bounds.is_empty());
        assert_eq!(l.panes[0].bounds.x, l.sidebar.right());
        assert_eq!(l.panes[0].bounds.right(), client.right());
        // Nothing on the right half can be hit.
        assert_eq!(l.hit_test(1300, 400), Hit::ListBackground(PaneId(0)));
    }

    #[test]
    fn pane_bands_tile_the_pane_exactly() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        assert_eq!(p.tab_bar.y, p.bounds.y);
        assert_eq!(p.tab_bar.bottom(), p.toolbar.y);
        assert_eq!(p.toolbar.bottom(), p.header.y);
        assert_eq!(p.header.bottom(), p.list.y);
        assert_eq!(p.list.bottom(), p.footer.y);
        assert_eq!(p.footer.bottom(), p.bounds.bottom());
        assert_eq!(p.list.right(), p.scrollbar.x);
        assert_eq!(p.scrollbar.right(), p.bounds.right());
    }

    #[test]
    fn clamp_split_keeps_both_panes_usable() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1000, 600);
        let low = Layout::clamp_splits(client, m, false, false, 2, &[-5.0]);
        let high = Layout::clamp_splits(client, m, false, false, 2, &[5.0]);
        assert_eq!((low[0] * 1000.0).round() as i32, m.min_pane_w);
        assert_eq!(
            (high[0] * 1000.0).round() as i32,
            1000 - m.min_pane_w - m.divider_w
        );
    }

    #[test]
    fn clamp_split_centres_when_the_window_is_too_narrow() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 200, 600);
        assert_eq!(Layout::clamp_splits(client, m, false, false, 2, &[0.05]), vec![0.5]);
    }

    // -- the bug this module was written to kill ---------------------------

    #[test]
    fn crumb_hit_test_matches_where_the_crumb_is_drawn() {
        let crumbs = strs(&["C:\\", "Users", "Sindroma", "Desktop"]);
        let l = build(1600, 800, &crumbs);
        assert_eq!(l.panes[0].crumbs.len(), 4, "all four should fit at this width");
        for c in &l.panes[0].crumbs {
            let cx = c.rect.x + c.rect.w / 2;
            let cy = c.rect.y + c.rect.h / 2;
            assert_eq!(
                l.hit_test(cx, cy),
                Hit::Crumb(PaneId(0), c.segment_index),
                "crumb {} drawn at {:?} did not hit-test to itself",
                c.segment_index,
                c.rect
            );
        }
    }

    #[test]
    fn crumbs_are_laid_out_by_measured_width_not_a_fixed_stride() {
        let crumbs = strs(&["C:\\", "a", "averyverylongfoldername"]);
        let l = build(1600, 800, &crumbs);
        let widths: Vec<i32> = l.panes[0].crumbs.iter().map(|c| c.rect.w).collect();
        assert!(
            widths[2] > widths[1] * 3,
            "long crumb must be much wider than a short one, got {:?}",
            widths
        );
    }

    #[test]
    fn crumbs_never_escape_the_breadcrumb_bar() {
        let crumbs = strs(&[
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "file-xplorer", "src", "deep",
        ]);
        let l = build(900, 800, &crumbs);
        for c in &l.panes[0].crumbs {
            assert!(
                c.rect.right() <= l.panes[0].breadcrumb.right(),
                "crumb {:?} spilled past {:?}",
                c.rect,
                l.panes[0].breadcrumb
            );
        }
    }

    #[test]
    fn crumbs_never_overlap_the_nav_buttons() {
        let l = build(900, 800, &strs(&["C:\\", "Users", "Sindroma"]));
        for c in &l.panes[0].crumbs {
            assert!(
                c.rect.x >= l.panes[0].nav_up.right(),
                "crumb {:?} overlaps the Up button {:?}",
                c.rect,
                l.panes[0].nav_up
            );
        }
    }

    #[test]
    fn overflowing_crumbs_elide_from_the_left_and_keep_the_leaf() {
        let crumbs = strs(&[
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "file-xplorer", "src",
        ]);
        let l = build(820, 800, &crumbs);
        let last = l.panes[0].crumbs.last().unwrap();
        assert_eq!(
            last.segment_index,
            crumbs.len() - 1,
            "the current folder must always be shown"
        );
        assert!(l.panes[0].crumbs[0].is_ellipsis, "hidden parents get an ellipsis");
    }

    #[test]
    fn nav_buttons_are_distinct_and_ordered() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        assert_eq!(p.nav_back.right(), p.nav_forward.x);
        assert_eq!(p.nav_forward.right(), p.nav_up.x);
        assert_eq!(
            l.hit_test(p.nav_back.x + 2, p.nav_back.y + 2),
            Hit::Nav(PaneId(0), NavButton::Back)
        );
        assert_eq!(
            l.hit_test(p.nav_up.x + 2, p.nav_up.y + 2),
            Hit::Nav(PaneId(0), NavButton::Up)
        );
    }

    #[test]
    fn tab_close_button_is_inside_its_tab_and_hit_tests_first() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Documents", "Downloads"]);
        let crumbs = strs(&["C:\\"]);
        let splits = Layout::clamp_splits(client, m, false, false, 2, &[]);
        let l = Layout::compute(
            client,
            m,
            &splits,
            false,
            &[],
            0,
            false,
            &[input(&tabs, &crumbs, 0, 0), input(&tabs, &crumbs, 0, 0)],
            &FixedWidth,
        );
        let t = l.panes[0].tabs[1];
        assert!(t.close.right() <= t.full.right());
        let cx = t.close.x + t.close.w / 2;
        let cy = t.close.y + t.close.h / 2;
        assert_eq!(l.hit_test(cx, cy), Hit::TabClose(PaneId(0), 1));
        assert_eq!(l.hit_test(t.full.x + 2, t.full.y + 2), Hit::Tab(PaneId(0), 1));
    }

    #[test]
    fn many_tabs_shrink_to_fit_instead_of_overflowing() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1000, 800);
        let tabs: Vec<String> = (0..12).map(|i| format!("Folder{}", i)).collect();
        let crumbs = strs(&["C:\\"]);
        let splits = Layout::clamp_splits(client, m, false, false, 2, &[]);
        let l = Layout::compute(
            client,
            m,
            &splits,
            false,
            &[],
            0,
            false,
            &[input(&tabs, &crumbs, 0, 0), input(&tabs, &crumbs, 0, 0)],
            &FixedWidth,
        );
        for t in &l.panes[0].tabs {
            assert!(
                t.full.right() <= l.panes[0].tab_bar.right() + 1,
                "tab {:?} overflowed the bar {:?}",
                t.full,
                l.panes[0].tab_bar
            );
        }
    }

    #[test]
    fn columns_drop_off_before_squeezing_the_name_column() {
        let wide = build_n(1, 1600, 800, &strs(&["C:\\"]));
        assert_eq!(wide.panes[0].columns.len(), 4);
        let narrow = build(760, 800, &strs(&["C:\\"]));
        assert!(narrow.panes[0].columns.len() < 4);
        assert_eq!(narrow.panes[0].columns[0].key, SortKey::Name);
        // Whatever survives, Name never drops below its floor.
        assert!(narrow.panes[0].columns[0].rect.w >= narrow.metrics.col_min_name_w);
    }

    #[test]
    fn a_column_edge_is_grabbable_and_does_not_swallow_the_header() {
        let l = build_n(1, 1600, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        let type_col = p.columns.iter().find(|c| c.key == SortKey::Type).unwrap();
        let y = p.header.y + p.header.h / 2;

        // On the boundary: resize.
        assert_eq!(
            l.hit_test(type_col.rect.x, y),
            Hit::ColumnEdge(PaneId(0), SortKey::Type)
        );
        // Well inside it: sort. Otherwise the header would stop being clickable.
        assert_eq!(
            l.hit_test(type_col.rect.x + type_col.rect.w / 2, y),
            Hit::Column(PaneId(0), SortKey::Type)
        );
        // Name has no edge of its own; the far left of the header sorts.
        assert_eq!(
            l.hit_test(p.header.x + 2, y),
            Hit::Column(PaneId(0), SortKey::Name)
        );
        // The strip scales with the monitor rather than being a fixed 4px.
        let hi = build_dpi(192, 1600, 800);
        let hi_col = hi.panes[0].columns.iter().find(|c| c.key == SortKey::Type).unwrap();
        assert_eq!(hi_col.edge.w, l.metrics.pad * 2);
    }

    /// One pane at an arbitrary DPI.
    fn build_dpi(dpi: u32, w: i32, h: i32) -> Layout {
        let m = Metrics::for_dpi(dpi);
        let client = Rect::new(0, 0, w, h);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        Layout::compute(
            client,
            m,
            &[],
            false,
            &[],
            0,
            false,
            &[input(&tabs, &crumbs, 10, 0)],
            &FixedWidth,
        )
    }

    #[test]
    fn size_is_the_last_optional_column_standing() {
        // A pane wide enough for exactly one extra column should spend it on
        // Size, not on Type or Date.
        let m = Metrics::for_dpi(96);
        let header = Rect::new(0, 0, m.col_min_name_w + m.col_size_w, m.header_h);
        let cols = column_rects(header, m);
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[1].key, SortKey::Size);
    }

    #[test]
    fn column_headers_tile_the_header_exactly() {
        let l = build(1600, 800, &strs(&["C:\\"]));
        let cols = &l.panes[0].columns;
        assert_eq!(cols[0].rect.x, l.panes[0].header.x);
        for pair in cols.windows(2) {
            assert_eq!(pair[0].rect.right(), pair[1].rect.x);
        }
        assert_eq!(cols.last().unwrap().rect.right(), l.panes[0].header.right());
    }

    #[test]
    fn row_hit_test_accounts_for_scroll() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let m = l.metrics;
        let p = &l.panes[0];
        let y = p.list.y + m.row_h * 2 + 1;
        // Scroll is in pixels, so ten rows down is ten row heights.
        assert_eq!(p.cell_at(p.list.x + 1, y, 10 * m.row_h, 100), Some(12));
        assert_eq!(p.cell_at(p.list.x + 1, y, 10 * m.row_h, 5), None);
        // Half a row scrolled: the same pixel is one row further down.
        assert_eq!(p.cell_at(p.list.x + 1, y, 10 * m.row_h + m.row_h / 2 + 1, 100), Some(12));
        assert_eq!(
            p.cell_at(p.list.x + 1, p.list.y + m.row_h - 1, m.row_h / 2 + 1, 100),
            Some(1),
            "a part-scrolled top row hands the rest of its band to the next"
        );
        // Below the list is not a row, even with plenty of data.
        assert_eq!(p.cell_at(p.list.x + 1, p.list.bottom() + 1, 0, 10_000), None);
    }

    #[test]
    fn scrollbar_thumb_reflects_position_and_inverts_cleanly() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let splits = Layout::clamp_splits(client, m, false, false, 2, &[]);
        let mk = |offset: u32| {
            Layout::compute(
                client,
                m,
                &splits,
                false,
                &[],
                0,
                false,
                &[
                    input(&tabs, &crumbs, 1000, offset),
                    input(&tabs, &crumbs, 1000, offset),
                ],
                &FixedWidth,
            )
        };
        let top = mk(0);
        assert_eq!(top.panes[0].thumb.y, top.panes[0].scrollbar.y);
        let visible = top.panes[0].visible_rows;
        let bottom = mk(1000 - visible);
        assert_eq!(
            bottom.panes[0].thumb.bottom(),
            bottom.panes[0].scrollbar.bottom()
        );

        let track = top.panes[0].scrollbar;
        assert_eq!(scroll_offset_for_thumb_y(track, m, 1000, visible, track.y), 0);
        assert_eq!(
            scroll_offset_for_thumb_y(track, m, 1000, visible, track.bottom()),
            1000 - visible
        );
    }

    #[test]
    fn no_scrollbar_thumb_when_everything_fits() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let splits = Layout::clamp_splits(client, m, false, false, 2, &[]);
        let l = Layout::compute(
            client,
            m,
            &splits,
            false,
            &[],
            0,
            false,
            &[input(&tabs, &crumbs, 3, 0), input(&tabs, &crumbs, 3, 0)],
            &FixedWidth,
        );
        assert!(l.panes[0].thumb.is_empty());
    }

    #[test]
    fn divider_hit_wins_over_the_panes() {
        let l = build_n(3, 1800, 800, &strs(&["C:\\"]));
        for (i, d) in l.dividers.iter().enumerate() {
            assert_eq!(l.hit_test(d.x + d.w / 2, 400), Hit::Divider(i));
        }
    }

    // -- sidebar -----------------------------------------------------------

    #[test]
    fn sidebar_rows_stack_without_overlapping() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let rows: Vec<&SidebarRow> = l.sidebar_rows.iter().filter(|r| !r.rect.is_empty()).collect();
        assert_eq!(rows.len(), 5); // section, 2 drives, section, place
        for pair in rows.windows(2) {
            assert_eq!(pair[0].rect.bottom(), pair[1].rect.y);
        }
    }

    #[test]
    fn drive_rows_are_taller_and_carry_a_capacity_bar() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let drive = &l.sidebar_rows[1];
        let place = l.sidebar_rows.last().unwrap();
        assert!(drive.rect.h > place.rect.h);
        assert!(!drive.capacity.is_empty());
        assert!(place.capacity.is_empty());
        assert!(
            drive.capacity.bottom() <= drive.rect.bottom(),
            "capacity bar escaped its row"
        );
    }

    #[test]
    fn capacity_bar_sits_below_the_label_not_across_it() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let m = l.metrics;
        let drive = &l.sidebar_rows[1];
        // Two text lines stack above the bar; the bar must clear both.
        let two_lines = drive.rect.y + m.pad / 2 + m.sidebar_row_h;
        assert!(
            drive.capacity.y >= two_lines,
            "bar at y={} runs through the text ending at y={}",
            drive.capacity.y,
            two_lines
        );
        assert!(drive.capacity.bottom() <= drive.rect.bottom());
        assert_eq!(drive.capacity.x, drive.rect.x + sidebar_text_offset(m));
    }

    #[test]
    fn section_chevron_hit_tests_before_the_row() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let s = &l.sidebar_rows[0];
        assert!(!s.chevron.is_empty());
        let cx = s.chevron.x + s.chevron.w / 2;
        let cy = s.chevron.y + s.chevron.h / 2;
        assert_eq!(l.hit_test(cx, cy), Hit::SidebarChevron(0));
        assert_eq!(l.hit_test(s.rect.x + 2, s.rect.y + 2), Hit::Sidebar(0));
    }

    #[test]
    fn sidebar_rows_past_the_bottom_are_empty_but_keep_their_index() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 140);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let entries: Vec<SidebarEntry> =
            (0..40).map(|i| SidebarEntry::Place { index: i }).collect();
        let splits = Layout::clamp_splits(client, m, true, false, 2, &[]);
        let l = Layout::compute(
            client,
            m,
            &splits,
            true,
            &entries,
            0,
            false,
            &[input(&tabs, &crumbs, 0, 0), input(&tabs, &crumbs, 0, 0)],
            &FixedWidth,
        );
        assert_eq!(l.sidebar_rows.len(), 40, "indices must still line up");
        assert!(l.sidebar_rows.last().unwrap().rect.is_empty());
    }

    #[test]
    fn footer_filter_hit_tests() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let f = l.panes[0].filter;
        assert!(!f.is_empty());
        assert_eq!(l.hit_test(f.x + 2, f.y + 2), Hit::Filter(PaneId(0)));
    }

    #[test]
    fn metrics_scale_with_dpi() {
        let lo = Metrics::for_dpi(96);
        let hi = Metrics::for_dpi(192);
        assert_eq!(hi.row_h, lo.row_h * 2);
        assert_eq!(hi.sidebar_w, lo.sidebar_w * 2);
        assert_eq!(hi.sidebar_drive_h, lo.sidebar_drive_h * 2);
    }

    #[test]
    fn zero_sized_window_does_not_panic() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 0, 0);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let l = Layout::compute(
            client,
            m,
            &[0.5],
            true,
            &sidebar_of(2),
            0,
            false,
            &[input(&tabs, &crumbs, 0, 0), input(&tabs, &crumbs, 0, 0)],
            &FixedWidth,
        );
        assert_eq!(l.hit_test(0, 0), Hit::Nothing);
    }

    #[test]
    fn band_selects_the_rows_it_covers() {
        let m = Metrics::for_dpi(96);
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        let x = p.list.x + 1;
        let top = p.list.y;
        let band = |y0: i32, y1: i32, scroll: i32| {
            p.band_indices(Rect::between((x, y0), (x, y1)), scroll, 100)
        };

        // A band over the first three rows, scrolled to the top.
        assert_eq!(band(top + 1, top + 2 * m.row_h + 1, 0), vec![0, 1, 2]);
        // Dragged upwards: same rows.
        assert_eq!(band(top + 2 * m.row_h + 1, top + 1, 0), vec![0, 1, 2]);
        // Scrolled: the same pixels mean later rows.
        assert_eq!(band(top + 1, top + 1, 10 * m.row_h), vec![10]);
    }

    #[test]
    fn band_clamps_rather_than_refusing() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        let x = p.list.x + 1;

        // Dragged far past the last row: stops at it.
        let covered = p.band_indices(
            Rect::between((x, p.list.y + 1), (x, p.list.bottom() + 5000)),
            0,
            3,
        );
        assert_eq!(covered, vec![0, 1, 2]);

        // Entirely above the rows, or over an empty listing: nothing.
        assert!(p
            .band_indices(Rect::between((x, 0), (x, p.list.y - 1)), 0, 3)
            .is_empty());
        assert!(p
            .band_indices(Rect::between((x, p.list.y), (x, p.list.bottom())), 0, 0)
            .is_empty());
    }

    #[test]
    fn between_and_clamp_to_normalise() {
        assert_eq!(Rect::between((10, 10), (4, 7)), Rect::new(4, 7, 6, 3));
        let clipped = Rect::new(0, 0, 100, 100).clamp_to(Rect::new(50, 50, 200, 200));
        assert_eq!(clipped, Rect::new(50, 50, 50, 50));
        assert!(Rect::new(0, 0, 10, 10).clamp_to(Rect::new(50, 50, 10, 10)).is_empty());
    }

    #[test]
    fn a_rows_name_cell_hit_tests_back_to_that_row() {
        let m = Metrics::for_dpi(96);
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        for row in [0u32, 1, 5] {
            let (icon, name) = p
                .cell_parts(row, 0, m)
                .expect("a row near the top is on screen");
            assert!(icon.right() < name.x, "the name starts after the icon");
            assert!(name.w > 0);
            // hit_test resolves the rows area; the row itself comes from
            // row_at, the way the input handlers do it.
            assert_eq!(
                l.hit_test(name.x + 1, name.y + name.h / 2),
                Hit::ListBackground(PaneId(0))
            );
            assert_eq!(
                p.cell_at(p.list.x + 1, name.y + name.h / 2, 0, 100),
                Some(row),
                "the cell drawn for a row belongs to that row"
            );
        }
    }

    #[test]
    fn a_half_visible_row_still_has_cells_to_paint() {
        // The bottom row is usually clipped. It must still be drawn, or its
        // name and icon go missing while its other columns render.
        let m = Metrics::for_dpi(96);
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        let partial = (p.list.h / m.row_h) as u32;
        assert!(
            p.list.h % m.row_h != 0,
            "this test needs a list height that does not divide evenly"
        );
        let (_, name) = p
            .cell_parts(partial, 0, m)
            .expect("a row hanging off the bottom edge is still painted");
        assert!(name.y < p.list.bottom() && name.bottom() > p.list.bottom());
    }

    #[test]
    fn a_scrolled_out_row_has_no_cell_to_edit() {
        let m = Metrics::for_dpi(96);
        let l = build(1400, 800, &strs(&["C:\\"]));
        let p = &l.panes[0];
        assert!(
            p.cell_parts(3, 10 * m.row_h, m).is_none(),
            "scrolled off the top"
        );
        let past = (p.list.h / m.row_h) as u32 + 5;
        assert!(p.cell_parts(past, 0, m).is_none(), "below the viewport");
    }

    #[test]
    fn the_inspector_takes_its_width_off_the_panes() {
        let m = Metrics::for_dpi(96);
        let tabs = strs(&["C:\\"]);
        let crumbs = strs(&["C:"]);
        let client = Rect::new(0, 0, 1400, 800);
        let mk = |inspector: bool| {
            Layout::compute(
                client,
                m,
                &[],
                true,
                &sidebar_of(2),
                0,
                inspector,
                &[input(&tabs, &crumbs, 10, 0)],
                &FixedWidth,
            )
        };

        let without = mk(false);
        assert!(without.inspector.is_empty());
        assert_eq!(without.panes[0].bounds.right(), client.right());

        let with = mk(true);
        assert_eq!(with.inspector.w, m.inspector_w);
        assert_eq!(with.inspector.right(), client.right());
        assert_eq!(with.inspector.h, client.h, "full height, like the sidebar");
        // The pane stops where the inspector starts: no overlap, no gap.
        assert_eq!(with.panes[0].bounds.right(), with.inspector.x);
        assert_eq!(
            with.panes[0].bounds.w,
            without.panes[0].bounds.w - m.inspector_w
        );
    }

    #[test]
    fn splits_are_clamped_against_the_body_the_inspector_leaves() {
        let m = Metrics::for_dpi(96);
        // Narrow enough that four panes plus an inspector cannot fit, but
        // four panes alone can: the clamp has to notice the difference.
        let client = Rect::new(0, 0, m.min_pane_w * 4 + m.divider_w * 3 + m.inspector_w + 20, 800);
        let even = Layout::clamp_splits(client, m, false, false, 4, &[0.1, 0.2, 0.3]);
        let squeezed = Layout::clamp_splits(client, m, false, true, 4, &[0.1, 0.2, 0.3]);
        assert_ne!(
            even, squeezed,
            "the same splits cannot mean the same pixels in a narrower body"
        );
    }

    fn grid_pane(w: i32) -> Layout {
        let tabs = strs(&["C:\\"]);
        let crumbs = strs(&["C:"]);
        Layout::compute(
            Rect::new(0, 0, w, 800),
            Metrics::for_dpi(96),
            &[],
            false,
            &[],
            0,
            false,
            &[PaneInput {
                tab_labels: &tabs,
                crumbs: &crumbs,
                total_lines: 50,
                scroll_offset: 0,
                grid: true,
            }],
            &FixedWidth,
        )
    }

    #[test]
    fn the_icon_view_tiles_cells_and_drops_the_header() {
        let m = Metrics::for_dpi(96);
        let l = grid_pane(1000);
        let p = &l.panes[0];
        assert!(p.header.is_empty(), "no columns, so no column header");
        assert_eq!(p.columns_per_line, (p.list.w / m.cell_w).max(1) as u32);
        assert_eq!(p.line_h, m.cell_h);
        assert!(p.columns_per_line > 1, "1000px should fit several cells");

        // The first line's cells sit side by side, all at the same height.
        let cols = p.columns_per_line;
        let first = p.cell(0, 0).unwrap();
        let second = p.cell(1, 0).unwrap();
        assert_eq!(first.y, second.y);
        assert_eq!(first.right(), second.x);
        // The next line starts one cell height down.
        assert_eq!(p.cell(cols, 0).unwrap().y, first.y + m.cell_h);
        assert_eq!(p.cell(cols, 0).unwrap().x, first.x);
    }

    #[test]
    fn a_cell_hit_tests_back_to_its_own_index() {
        let l = grid_pane(1000);
        let p = &l.panes[0];
        for index in [0u32, 1, 5, 12] {
            let c = p.cell(index, 0).expect("near the top, so on screen");
            assert_eq!(
                p.cell_at(c.x + c.w / 2, c.y + c.h / 2, 0, 100),
                Some(index),
                "whatever is drawn at a cell must hit-test back to it"
            );
        }
    }

    #[test]
    fn the_strip_past_the_last_column_belongs_to_no_cell() {
        let l = grid_pane(1000);
        let p = &l.panes[0];
        // cell_at divides the list width by the column count, so every pixel
        // of the list falls inside a column; what must not happen is a click
        // past the last *entry* on a line selecting something.
        let cols = p.columns_per_line;
        let last_on_line = p.cell(cols - 1, 0).unwrap();
        assert_eq!(
            p.cell_at(last_on_line.x + 1, last_on_line.y + 1, 0, cols - 1),
            None,
            "a line that is not full has empty cells at the end"
        );
        assert!(p.cell_at(p.list.x + 1, p.list.y - 5, 0, 100).is_none());
    }

    #[test]
    fn a_band_over_a_grid_takes_only_the_cells_it_touches() {
        let l = grid_pane(1000);
        let p = &l.panes[0];
        let cols = p.columns_per_line;
        let first = p.cell(0, 0).unwrap();
        let second = p.cell(1, 0).unwrap();

        // A band over the first two cells of line one takes exactly those two,
        // not the whole line — which is the difference between a grid band and
        // the row range the details view gets.
        let band = Rect::between(
            (first.x + 2, first.y + 2),
            (second.x + 2, second.bottom() - 2),
        );
        assert_eq!(p.band_indices(band, 0, 100), vec![0, 1]);
        assert!(cols > 2, "this test needs a line wider than the band");
    }
}
