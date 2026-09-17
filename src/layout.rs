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
    /// One of the three window buttons at the right end of the caption. The
    /// size Windows draws its own at, so the target is where the hand expects.
    pub win_btn_w: i32,
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
    pub min_pane_h: i32,
    pub icon_size: i32,
    /// One cell of the icon view, and the icon drawn inside it. Wide enough
    /// for a readable two-line name under a thumbnail.
    /// Icon sizes the view steps through, in DIPs. Smallest first; stepping
    /// below the first one is the details list.
    pub icon_steps: &'static [i32],
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
    /// Metrics at 100% text and normal density, which is what almost every
    /// caller wants and every test does.
    pub fn for_dpi(dpi: u32) -> Self {
        Self::sized(dpi, 100, 100)
    }

    /// `font` and `density` are percentages.
    ///
    /// Anything a line of text sits in follows both: bigger text in a row that
    /// did not grow is clipped text, which is why density cannot shrink a row
    /// below what the font needs. Gaps follow density alone — that is the
    /// part that is taste rather than legibility.
    pub fn sized(dpi: u32, font: i32, density: i32) -> Self {
        let scale = dpi as f32 / 96.0;
        let font = (font.clamp(FONT_MIN, FONT_MAX) as f32) / 100.0;
        let density = (density.clamp(DENSITY_MIN, DENSITY_MAX) as f32) / 100.0;
        // Rows: the text's scale, then density on top of it.
        let s = |v: f32| (v * scale * font * density).round() as i32;
        // Fixed furniture: chrome that holds no text, or whose size is a
        // pointer target rather than a reading distance.
        let f = |v: f32| (v * scale).round() as i32;
        // Gaps.
        let g = |v: f32| (v * scale * density).round() as i32;
        // Text, without density: for the one row that is already full. Density
        // takes back the slack around a line of text, and a drive row has none
        // — two lines and a capacity bar, stacked.
        let t = |v: f32| (v * scale * font).round() as i32;
        Self {
            scale,

            tab_bar_h: s(29.0),
            tab_min_w: s(96.0),
            tab_max_w: s(190.0),
            tab_close_w: f(18.0),
            new_tab_w: f(26.0),

            toolbar_h: s(29.0),
            win_btn_w: f(46.0),
            nav_btn_w: f(26.0),
            header_h: s(24.0),
            footer_h: s(24.0),
            // Dense by design: this is a list you scan, not a form you read.
            row_h: s(22.0),

            sidebar_w: s(196.0),
            inspector_w: s(300.0),
            sidebar_section_h: s(26.0),
            sidebar_row_h: s(24.0),
            sidebar_drive_h: t(46.0),
            capacity_bar_h: f(3.0),

            divider_w: f(6.0),
            scrollbar_w: f(11.0),
            scrollbar_min_thumb: f(28.0),
            min_pane_w: s(240.0),
            min_pane_h: s(140.0),
            icon_size: s(16.0),
            icon_steps: ICON_STEPS,
            pad: g(8.0),
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

/// How the body is divided between panes.
///
/// A tree rather than a list of fractions, because a pane is no longer "the
/// n-th column from the left": it is a leaf, and where it sits is what the tree
/// above it says. `PaneId` keeps meaning what it always meant — an index into
/// the caller's pane array — so tabs, sessions and settings are untouched.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split {
        /// True when the dividing line is vertical, i.e. the children sit side
        /// by side. False stacks them.
        vertical: bool,
        /// Where the line falls across this node, 0..1.
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// One dividing line, and which way it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub vertical: bool,
}

impl Node {
    /// `n` panes side by side in even columns: what every layout was before
    /// there were trees, and what Ctrl+1..4 still builds.
    pub fn columns(n: usize) -> Node {
        let n = n.clamp(1, MAX_PANES);
        let mut node = Node::Leaf(PaneId(n - 1));
        // Built from the right so the ratios come out even: the first split
        // gives away 1/n, the next 1/(n-1) of what is left, and so on.
        for i in (0..n - 1).rev() {
            let remaining = (n - i) as f32;
            node = Node::Split {
                vertical: true,
                ratio: 1.0 / remaining,
                a: Box::new(Node::Leaf(PaneId(i))),
                b: Box::new(node),
            };
        }
        node
    }

    /// Every pane in the tree, left to right and top to bottom. The order Tab
    /// cycles in, and the order the caller's `visible()` reports.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect_leaves(out);
                b.collect_leaves(out);
            }
        }
    }

    pub fn count(&self) -> usize {
        self.leaves().len()
    }

    /// Ids not used by any leaf, so a new pane can take one without colliding.
    pub fn unused(&self) -> Vec<PaneId> {
        let used = self.leaves();
        (0..MAX_PANES)
            .map(PaneId)
            .filter(|p| !used.contains(p))
            .collect()
    }

    /// Replace the leaf for `id` with a split holding it and `with`.
    /// `vertical` puts the new pane to the right, otherwise underneath;
    /// `before` puts it on the left, or on top: which side a dropped tab
    /// landed on.
    pub fn split_at(&mut self, id: PaneId, with: PaneId, vertical: bool, before: bool) -> bool {
        match self {
            Node::Leaf(mine) if *mine == id => {
                let (a, b) = if before { (with, id) } else { (id, with) };
                *self = Node::Split {
                    vertical,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(a)),
                    b: Box::new(Node::Leaf(b)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                a.split_at(id, with, vertical, before) || b.split_at(id, with, vertical, before)
            }
        }
    }

    /// Drop a pane, collapsing the split that held it. The last pane cannot go.
    pub fn close(&mut self, id: PaneId) -> bool {
        if self.count() < 2 {
            return false;
        }
        self.close_inner(id)
    }

    fn close_inner(&mut self, id: PaneId) -> bool {
        let replacement = match self {
            Node::Leaf(_) => return false,
            Node::Split { a, b, .. } => match (a.as_ref(), b.as_ref()) {
                (Node::Leaf(x), _) if *x == id => (**b).clone(),
                (_, Node::Leaf(y)) if *y == id => (**a).clone(),
                _ => {
                    return a.close_inner(id) || b.close_inner(id);
                }
            },
        };
        *self = replacement;
        true
    }

    /// Set the ratio of the `index`-th split, in the order `place` produces
    /// dividers. Keeping dividers addressed by position is what lets the drag
    /// handler stay as it was.
    pub fn set_ratio(&mut self, index: usize, ratio: f32) {
        let mut seen = 0;
        self.set_ratio_inner(index, ratio, &mut seen);
    }

    fn set_ratio_inner(&mut self, index: usize, ratio: f32, seen: &mut usize) {
        if let Node::Split { a, b, ratio: r, .. } = self {
            // Depth-first, own split before children, matching `place`.
            if *seen == index {
                *r = ratio.clamp(0.0, 1.0);
                *seen += 1;
                return;
            }
            *seen += 1;
            a.set_ratio_inner(index, ratio, seen);
            b.set_ratio_inner(index, ratio, seen);
        }
    }

    /// Lay the tree out in `rect`, producing one rectangle per pane and one per
    /// divider.
    ///
    /// Minimum sizes are enforced here rather than by clamping the ratios
    /// beforehand: a ratio is a fraction of whatever space its node actually
    /// got, and only placement knows what that was.
    pub fn place(&self, rect: Rect, m: Metrics) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        place_into(self, rect, m, &mut panes, &mut dividers);
        (panes, dividers)
    }
}

fn place_into(
    node: &Node,
    rect: Rect,
    m: Metrics,
    panes: &mut Vec<(PaneId, Rect)>,
    dividers: &mut Vec<Divider>,
) {
    match node {
        Node::Leaf(id) => panes.push((*id, rect)),
        Node::Split {
            vertical,
            ratio,
            a,
            b,
        } => {
            let (span, min) = if *vertical {
                (rect.w, m.min_pane_w)
            } else {
                (rect.h, m.min_pane_h)
            };
            if span < min * 2 + m.divider_w {
                // No room to divide: the first child takes it all. Better than
                // two panes too narrow to use.
                place_into(a, rect, m, panes, dividers);
                return;
            }
            let at = ((span - m.divider_w) as f32 * ratio).round() as i32;
            let at = at.clamp(min, span - min - m.divider_w);
            let (first, divider, second) = if *vertical {
                (
                    Rect::new(rect.x, rect.y, at, rect.h),
                    Rect::new(rect.x + at, rect.y, m.divider_w, rect.h),
                    Rect::new(
                        rect.x + at + m.divider_w,
                        rect.y,
                        rect.w - at - m.divider_w,
                        rect.h,
                    ),
                )
            } else {
                (
                    Rect::new(rect.x, rect.y, rect.w, at),
                    Rect::new(rect.x, rect.y + at, rect.w, m.divider_w),
                    Rect::new(
                        rect.x,
                        rect.y + at + m.divider_w,
                        rect.w,
                        rect.h - at - m.divider_w,
                    ),
                )
            };
            dividers.push(Divider {
                rect: divider,
                vertical: *vertical,
            });
            place_into(a, first, m, panes, dividers);
            place_into(b, second, m, panes, dividers);
        }
    }
}

/// Which pane, not where it is: the layout tree says where. Stable across a
/// close, so closing the leftmost pane does not renumber the rest.
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
    /// The user's own folder. One click for the place every other one hangs
    /// off, which the sidebar has but a hidden sidebar does not.
    Home,
    /// Pin or unpin the folder showing. A toggle, so it also says whether
    /// this folder is already pinned.
    Pin,
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
    pub nav_home: Rect,
    pub nav_pin: Rect,
    /// The menu button at the right end of the breadcrumb row.
    pub overflow: Rect,
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
    /// Icon edge inside a cell, already in physical pixels.
    pub cell_icon: i32,
    /// Is a cell's label beside its icon rather than under it?
    pub side_label: bool,

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
        if self.side_label && self.columns_per_line > 1 {
            // Tiles and List: the icon at the left of the cell, the name on
            // one line beside it, both centred against the cell's height.
            let icon = Rect::new(
                cell.x + m.pad / 2,
                cell.y + (cell.h - self.cell_icon) / 2,
                self.cell_icon,
                self.cell_icon,
            );
            let label = Rect::new(
                icon.right() + m.pad / 2,
                cell.y + (cell.h - m.row_h) / 2,
                (cell.right() - icon.right() - m.pad).max(0),
                m.row_h,
            );
            return Some((icon, label));
        }
        if self.columns_per_line > 1 {
            // Icon view: a big icon over a two-line name, both centred.
            let icon = Rect::new(
                cell.x + (cell.w - self.cell_icon) / 2,
                cell.y + m.pad,
                self.cell_icon,
                self.cell_icon,
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
    pub sidebar: Rect,
    pub sidebar_rows: Vec<SidebarRow>,
    /// The inspector panel, or empty when it is hidden. Mirrors the sidebar:
    /// taken off the body before the panes are laid out, so nothing else has
    /// to know it exists.
    pub inspector: Rect,
    /// The strip across the top of the window that used to be the system
    /// caption. The panes on the top row put their tab bars in it, which is
    /// what makes the chrome two rows instead of four.
    pub caption: Rect,
    /// The app icon at the caption's left end. Not a button: it is there to
    /// say which window this is, and to be dragged by.
    pub caption_logo: Rect,
    /// The sidebar toggle, beside the logo.
    pub caption_sidebar: Rect,
    pub win_min: Rect,
    pub win_max: Rect,
    pub win_close: Rect,
    /// Grab strips on the sidebar's right edge and the inspector's left one.
    /// Empty when that panel is hidden; there is no edge to pull then.
    pub sidebar_edge: Rect,
    pub inspector_edge: Rect,
    /// Indexed by `PaneId`, always `MAX_PANES` long. A pane not in the tree
              /// gets an empty layout, which draws nothing and hit-tests to nothing.
    pub panes: Vec<PaneLayout>,
    /// One fewer than there are panes. Empty when one pane fills the window.
    pub dividers: Vec<Divider>,
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
    /// The sidebar toggle in the caption.
    SidebarToggle,
    /// The draggable edge of the sidebar (true) or the inspector (false).
    PanelEdge { sidebar: bool },
    /// Anywhere in the inspector panel. It has nothing to click, but a
    /// right-click there is a question about whatever it is describing.
    Inspector,
    /// The menu at the right end of a pane's breadcrumb row: everything the
    /// command bar used to hold, for the pane it sits on.
    Overflow(PaneId),
}

/// Where a dragged tab would land if it were dropped now.
///
/// `Into` is the pane itself — another tab in its strip. The rest each make
/// a new pane on that side of the one under the cursor, which is what the tree
/// has been able to do since panes stopped being a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropZone {
    Into,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    /// `(vertical, before)` for `Node::split_at`. None for `Into`, which is
    /// not a split at all.
    pub fn split(self) -> Option<(bool, bool)> {
        match self {
            DropZone::Into => None,
            DropZone::Left => Some((true, true)),
            DropZone::Right => Some((true, false)),
            DropZone::Top => Some((false, true)),
            DropZone::Bottom => Some((false, false)),
        }
    }
}

/// How much of a pane's span, from each edge, drops as a split rather than in.
/// A third is enough to aim at without making the middle hard to hit.
const DROP_EDGE: f32 = 0.33;

/// Everything the layout needs to know about one pane to size it.
/// From a thumbnail you can just about tell a photo apart, to one you can read
/// a screenshot in. Doubling roughly each step, so a wheel notch is a visible
/// change rather than a nudge.
pub const ICON_STEPS: &[i32] = &[32, 48, 72, 104, 144, 192, 256];

/// Text size and density, as percentages. The bounds are where the layout
/// still works: below the minimum the columns stop fitting their headers,
/// above the maximum a row is taller than the list.
pub const FONT_MIN: i32 = 80;
pub const FONT_MAX: i32 = 200;
pub const DENSITY_MIN: i32 = 80;
pub const DENSITY_MAX: i32 = 140;
/// The sizes offered, rather than a free number: every one of these is a step
/// somebody can see.
pub const FONT_STEPS: &[i32] = &[80, 90, 100, 110, 125, 150, 175, 200];
pub const DENSITY_STEPS: &[(&str, i32)] =
    &[("Compact", 85), ("Normal", 100), ("Roomy", 120)];

/// The details list, as an icon size. Zero is not a size, which is what makes
/// "one scalar decides the view" work: no separate flag to keep in step.
pub const ICONS_OFF: i32 = 0;

/// A negative size is the same grid with the label *beside* the icon rather
/// than under it: Tiles when the icon is big, List when it is small. Still one
/// scalar, because everything that remembers a view — the settings file, the
/// per-folder map, the layout's input — already carries exactly one, and a
/// second flag beside it is a second thing to keep in step.
pub const TILES: i32 = -72;
pub const LIST: i32 = -32;

/// Is the label beside the icon, and how big is that icon?
pub fn beside(icons: i32) -> bool {
    icons < 0
}

impl Metrics {
    /// Cell width, height and icon edge for an icon view at `icon_dip`.
    ///
    /// A cell is the icon plus room for two lines of name; both constants were
    /// the difference between the old fixed 72px icon and its 118x124 cell, so
    /// the default size still lays out exactly as it did.
    pub fn cell(&self, icon_dip: i32) -> (i32, i32, i32) {
        let px = |v: i32| (v as f32 * self.scale).round() as i32;
        let icon = px(icon_dip);
        (icon + px(46), icon + px(52), icon)
    }

    /// The same, with the label beside the icon: a row of icon-then-name,
    /// wide enough for a name at any icon size, and never shorter than a
    /// details row.
    pub fn cell_beside(&self, icon_dip: i32) -> (i32, i32, i32) {
        let px = |v: i32| (v as f32 * self.scale).round() as i32;
        let icon = px(icon_dip);
        (icon + px(200), (icon + px(12)).max(self.row_h), icon)
    }

    /// The next size up or down, or `ICONS_OFF` when stepping below the
    /// smallest. Stepping up from the details list enters at the default.
    /// Where the label sits is not a size, so the step keeps it.
    pub fn step_icons(&self, current: i32, up: bool) -> i32 {
        let steps = self.icon_steps;
        if current == ICONS_OFF {
            return if up { DEFAULT_ICONS } else { ICONS_OFF };
        }
        let sign = if beside(current) { -1 } else { 1 };
        let i = steps.iter().position(|s| *s == current.abs()).unwrap_or(0);
        match up {
            true => steps[(i + 1).min(steps.len() - 1)] * sign,
            false if i == 0 => ICONS_OFF,
            false => steps[i - 1] * sign,
        }
    }
}

/// What the icon view opens at: big enough to recognise a photo, small enough
/// that a folder of them still fits on a screen.
pub const DEFAULT_ICONS: i32 = 72;

pub struct PaneInput<'a> {
    pub tab_labels: &'a [String],
    /// Which tab is showing. The strip scrolls to keep it on screen.
    pub active_tab: usize,
    pub crumbs: &'a [String],
    /// Lines of cells, not entries: in the icon view one line holds several.
    /// It is what the scrollbar measures against.
    pub total_lines: u32,
    pub scroll_offset: u32,
    /// Icon edge in DIPs, or `ICONS_OFF` for the details list.
    pub icons: i32,
}

impl Layout {
    /// Compute the whole frame.
    ///
    /// `tree` carries a fraction per split rather than pixels, so resizing the
    /// window keeps the panes in proportion instead of squeezing the last one.
    pub fn compute(
        client: Rect,
        metrics: Metrics,
        tree: &Node,
        sidebar_visible: bool,
        sidebar_entries: &[SidebarEntry],
        // How far the sidebar is scrolled, in pixels.
        sidebar_scroll: i32,
        inspector_visible: bool,
        panes: &[PaneInput],
        measurer: &dyn TextMeasurer,
    ) -> Layout {
        let m = metrics;
        // The caption is not split off the top: the panes run up into it, and
        // a pane whose bounds start at the window's top edge puts its tab bar
        // there by itself. All this strip does is reserve the two ends — the
        // logo and toggle on the left, the window buttons on the right — and
        // give the painter and the caption drag something to ask about.
        let (caption, _) = client.split_top(m.tab_bar_h);
        let (caption_logo, r) = caption.split_left(m.nav_btn_w + m.pad);
        let (caption_sidebar, _) = r.split_left(m.nav_btn_w);
        let buttons_w = m.win_btn_w * 3;
        let win_min = Rect::new(caption.right() - buttons_w, caption.y, m.win_btn_w, caption.h);
        let win_max = Rect::new(win_min.right(), caption.y, m.win_btn_w, caption.h);
        let win_close = Rect::new(win_max.right(), caption.y, m.win_btn_w, caption.h);

        // The sidebar and the inspector start below the caption; the body does
        // not, because that is where the tabs go.
        let (sidebar, body) = if sidebar_visible {
            let (s, b) = client.split_left(m.sidebar_w);
            (s.split_top(m.tab_bar_h).1, b)
        } else {
            (Rect::default(), client)
        };
        let (body, inspector) = if inspector_visible {
            let (b, i) = body.split_right(m.inspector_w);
            (b, i.split_top(m.tab_bar_h).1)
        } else {
            (body, Rect::default())
        };
        // How far a top-row pane's tab strip must keep clear of each end. When
        // the sidebar or the inspector is showing it is already clear of them,
        // and the subtraction goes negative — which is the same answer.
        let tab_inset = (
            (caption_sidebar.right() - body.x).max(0),
            (body.right() - win_min.x).max(0),
        );

        // The grab strip lies inside the panel rather than across the line:
        // straddling it would put three pixels of a resize handle over the
        // first pane's Back button, which is a control you aim at.
        let edge_of = |r: Rect, right_side: bool| {
            if r.is_empty() || r.w < m.divider_w {
                return Rect::default();
            }
            let x = if right_side { r.right() - m.divider_w } else { r.x };
            Rect::new(x, r.y, m.divider_w, r.h)
        };
        let sidebar_edge = edge_of(sidebar, true);
        let inspector_edge = edge_of(inspector, false);

        let sidebar_rows = sidebar_rows(sidebar, m, sidebar_entries, sidebar_scroll);
        let (placed, dividers) = tree.place(body, m);

        Layout {
            metrics: m,
            sidebar,
            sidebar_rows,
            inspector,
            caption,
            caption_logo,
            caption_sidebar,
            win_min,
            win_max,
            win_close,
            sidebar_edge,
            inspector_edge,
            // Indexed by id, not by position: with a tree the two are no
            // longer the same thing. The caller's inputs arrive in leaf order,
            // which is the order `place` produced them in.
            panes: {
                let mut out = vec![PaneLayout::default(); MAX_PANES];
                for ((id, rect), input) in placed.iter().zip(panes) {
                    // Only the row of panes that reaches the top edge shares
                    // the caption, so only it has ends to keep clear of.
                    let inset = if rect.y <= client.y { tab_inset } else { (0, 0) };
                    if let Some(slot) = out.get_mut(id.0) {
                        *slot = pane_layout(*rect, m, input, inset, measurer);
                    }
                }
                out
            },
            dividers,
            fallback: PaneLayout::default(),
        }
    }

    /// Where a divider dragged to (`x`, `y`) falls within the space its own
    /// split node owns, as a fraction.
    ///
    /// The node's space is the two panes either side of it plus the divider,
    /// which is the whole body only when there is one divider — so this asks
    /// the rectangles rather than assuming, the way it used to.
    pub fn split_fraction_at(&self, index: usize, x: i32, y: i32) -> f32 {
        let Some(d) = self.dividers.get(index) else {
            return 0.5;
        };
        let (lo, hi, at) = if d.vertical {
            let (mut lo, mut hi) = (d.rect.x, d.rect.right());
            for p in self.panes.iter().filter(|p| !p.bounds.is_empty()) {
                let b = p.bounds;
                let overlaps = b.y < d.rect.bottom() && b.bottom() > d.rect.y;
                if overlaps && b.right() == d.rect.x {
                    lo = lo.min(b.x);
                }
                if overlaps && b.x == d.rect.right() {
                    hi = hi.max(b.right());
                }
            }
            (lo, hi, x)
        } else {
            let (mut lo, mut hi) = (d.rect.y, d.rect.bottom());
            for p in self.panes.iter().filter(|p| !p.bounds.is_empty()) {
                let b = p.bounds;
                let overlaps = b.x < d.rect.right() && b.right() > d.rect.x;
                if overlaps && b.bottom() == d.rect.y {
                    lo = lo.min(b.y);
                }
                if overlaps && b.y == d.rect.bottom() {
                    hi = hi.max(b.bottom());
                }
            }
            (lo, hi, y)
        };
        let span = (hi - lo - self.metrics.divider_w).max(1);
        ((at - lo) as f32 / span as f32).clamp(0.0, 1.0)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        if self.caption_sidebar.contains(x, y) {
            return Hit::SidebarToggle;
        }
        // Before the panels themselves, or the edge would be a sidebar row.
        if self.sidebar_edge.contains(x, y) {
            return Hit::PanelEdge { sidebar: true };
        }
        if self.inspector_edge.contains(x, y) {
            return Hit::PanelEdge { sidebar: false };
        }
        for (i, d) in self.dividers.iter().enumerate() {
            if d.rect.contains(x, y) {
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
        if self.inspector.contains(x, y) {
            return Hit::Inspector;
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

    /// Which pane a tab dropped at `(x, y)` would land in, where in it, and
    /// the rectangle that half would occupy.
    ///
    /// The nearest edge wins, so a corner goes to whichever side it is closer
    /// to rather than to whichever test ran first. Past a third of the way in
    /// from every edge there is no nearest edge worth having, and the drop is
    /// into the pane itself.
    pub fn drop_zone(&self, x: i32, y: i32) -> Option<(PaneId, DropZone, Rect)> {
        let pid = self.pane_at(x, y)?;
        let r = self.pane(pid).bounds;
        if r.w <= 0 || r.h <= 0 {
            return None;
        }
        // A tab bar is not an edge to split against: it sits inside the
        // pane's top third, so without this a five-pixel twitch while
        // clicking a tab drops it there and splits the pane in two.
        if self.pane(pid).tab_bar.contains(x, y) {
            return Some((pid, DropZone::Into, r));
        }
        let (w, h) = (r.w as f32, r.h as f32);
        let near = [
            (DropZone::Left, (x - r.x) as f32 / w),
            (DropZone::Right, (r.right() - x) as f32 / w),
            (DropZone::Top, (y - r.y) as f32 / h),
            (DropZone::Bottom, (r.bottom() - y) as f32 / h),
        ];
        let (zone, d) = near
            .iter()
            .copied()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((DropZone::Into, 1.0));
        let zone = if d < DROP_EDGE { zone } else { DropZone::Into };

        let half = match zone {
            DropZone::Into => r,
            DropZone::Left => Rect::new(r.x, r.y, r.w / 2, r.h),
            DropZone::Right => Rect::new(r.x + r.w / 2, r.y, r.w - r.w / 2, r.h),
            DropZone::Top => Rect::new(r.x, r.y, r.w, r.h / 2),
            DropZone::Bottom => Rect::new(r.x, r.y + r.h / 2, r.w, r.h - r.h / 2),
        };
        Some((pid, zone, half))
    }
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
        if p.overflow.contains(x, y) {
            return Some(Hit::Overflow(pid));
        }
        if p.nav_back.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Back));
        }
        if p.nav_forward.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Forward));
        }
        if p.nav_up.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Up));
        }
        if p.nav_home.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Home));
        }
        if p.nav_pin.contains(x, y) {
            return Some(Hit::Nav(pid, NavButton::Pin));
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

/// Lay the command bar out: icon-only buttons at a fixed width, labelled ones
/// as wide as their text needs, and the right-aligned ones packed from the far
/// end. Anything that does not fit gets an empty rectangle rather than
/// overlapping its neighbour.
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
    // Pixels the tab strip keeps clear at each end, for a pane sharing the
    // caption row with the window's own furniture.
    tab_inset: (i32, i32),
    measurer: &dyn TextMeasurer,
) -> PaneLayout {
    if bounds.w <= 0 || bounds.h <= 0 {
        return PaneLayout {
            bounds,
            ..Default::default()
        };
    }

    let (strip, rest) = bounds.split_top(m.tab_bar_h);
    let tab_bar = Rect::new(
        strip.x + tab_inset.0,
        strip.y,
        (strip.w - tab_inset.0 - tab_inset.1).max(0),
        strip.h,
    );
    let (toolbar, rest) = rest.split_top(m.toolbar_h);
    // The icon view has no columns, so it has no column header to click.
    let grid = input.icons != ICONS_OFF;
    let (header, rest) = rest.split_top(if grid { 0 } else { m.header_h });
    let (list_area, footer) = rest.split_bottom(m.footer_h);
    let (list, scrollbar) = list_area.split_right(m.scrollbar_w);

    let tabs = tab_rects(tab_bar, m, input.tab_labels, input.active_tab, measurer);
    let new_tab = {
        // The last tab actually on screen, not the last tab there is.
        let after = tabs
            .iter()
            .rev()
            .find(|t| !t.full.is_empty())
            .map(|t| t.full.right())
            .unwrap_or(tab_bar.x);
        let x = after.min(tab_bar.right() - m.new_tab_w);
        if x >= tab_bar.x && tab_bar.w >= m.new_tab_w {
            Rect::new(x, tab_bar.y, m.new_tab_w, tab_bar.h)
        } else {
            Rect::default()
        }
    };

    // Toolbar, left to right: the nav buttons, then the breadcrumb filling
    // what is left and the overflow menu off the far end.
    let (r, overflow) = toolbar.split_right(m.nav_btn_w);
    let (nav_back, r) = r.split_left(m.nav_btn_w);
    let (nav_forward, r) = r.split_left(m.nav_btn_w);
    let (nav_up, r) = r.split_left(m.nav_btn_w);
    let (nav_home, r) = r.split_left(m.nav_btn_w);
    let (nav_pin, breadcrumb) = r.split_left(m.nav_btn_w);
    let crumbs = crumb_rects(breadcrumb, m, input.crumbs, measurer);

    let columns = column_rects(header, m);

    // Details is the grid with one column: everything below works off these
    // two numbers, so neither the painter nor hit-testing needs a view branch.
    let side_label = beside(input.icons);
    let (cell_w, cell_h, cell_icon) = if side_label {
        m.cell_beside(-input.icons)
    } else {
        m.cell(input.icons)
    };
    let (columns_per_line, line_h, cell_icon) = if grid {
        ((list.w / cell_w.max(1)).max(1) as u32, cell_h, cell_icon)
    } else {
        (1, m.row_h, m.icon_size)
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
        nav_home,
        nav_pin,
        overflow,
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
        cell_icon,
        side_label,
        footer,
        filter: filter.inset(m.pad / 2, m.pad / 4),
        counts,
    }
}

/// Where each tab goes, and which of them are on screen at all.
///
/// The returned list always has one entry per label, so index `i` is tab `i`
/// whatever is showing; a tab scrolled out of the strip gets an empty
/// rectangle, which draws nothing and hit-tests to nothing.
fn tab_rects(
    bar: Rect,
    m: Metrics,
    labels: &[String],
    active: usize,
    measurer: &dyn TextMeasurer,
) -> Vec<TabRect> {
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
    // Shrink proportionally when they will not all fit, but only down to a
    // width that still shows an icon and a letter or two. Past that there are
    // simply more tabs than fit, and forty slivers with a hairline between
    // them is a strip you cannot read, aim at or close anything in.
    // Enough for the icon and four or five letters. Below that every tab in a
    // folder tree reads "D…" and the strip stops telling you anything.
    let floor = m.tab_min_w * 3 / 4;
    let widths: Vec<i32> = if total <= available || total == 0 {
        natural
    } else {
        natural
            .iter()
            .map(|w| ((*w as i64 * available as i64) / total as i64).max(floor as i64) as i32)
            .collect()
    };

    // More than fit: scroll the strip so the active tab is in it. Switching
    // tabs is what moves the window along — Ctrl+Tab walks to a tab that is
    // off the end and brings it into view, which is the whole affordance.
    let active = active.min(labels.len() - 1);
    let mut first = 0;
    while first < active
        && widths[first..=active].iter().sum::<i32>() > available
    {
        first += 1;
    }

    let mut out = vec![TabRect { full: Rect::default(), close: Rect::default() }; labels.len()];
    let mut x = bar.x;
    for (i, w) in widths.iter().enumerate().skip(first) {
        if x + w > bar.right() && i != first {
            break;
        }
        let full = Rect::new(x, bar.y, (*w).min((bar.right() - x).max(0)), bar.h);
        if full.w <= 0 {
            break;
        }
        // A squeezed tab drops its close button rather than its name: the
        // name is the only thing telling the tabs apart, and Ctrl+W still
        // closes one whether or not it is drawing an x.
        let close = if full.w > m.tab_min_w {
            Rect::new(
                full.right() - m.tab_close_w - m.pad / 2,
                full.y + (full.h - m.tab_close_w) / 2,
                m.tab_close_w,
                m.tab_close_w,
            )
        } else {
            Rect::default()
        };
        out[i] = TabRect { full, close };
        x += w;
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
            active_tab: 0,
            crumbs,
            total_lines: rows,
            icons: ICONS_OFF,
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
        build_tree(&Node::columns(n), w, h, crumbs)
    }

    fn build_tree(tree: &Node, w: i32, h: i32, crumbs: &[String]) -> Layout {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, w, h);
        let tabs = strs(&["Foo"]);
        let inputs: Vec<PaneInput> = (0..tree.count())
            .map(|_| input(&tabs, crumbs, 100, 0))
            .collect();
        Layout::compute(client, m, tree, true, &sidebar_of(2), 0, false, &inputs, &FixedWidth)
    }

    /// Panes that are actually on screen, in tree order.
    fn placed(l: &Layout, tree: &Node) -> Vec<Rect> {
        tree.leaves().iter().map(|p| l.pane(*p).bounds).collect()
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
            let tree = Node::columns(n);
            let l = build_tree(&tree, 1800, 800, &strs(&["C:\\", "Users"]));
            let b = placed(&l, &tree);
            assert_eq!(b.len(), n);
            assert_eq!(l.dividers.len(), n - 1);
            assert_eq!(b[0].x, l.sidebar.right(), "n={}", n);
            for i in 0..n - 1 {
                assert_eq!(b[i].right(), l.dividers[i].rect.x, "n={}", n);
                assert_eq!(l.dividers[i].rect.right(), b[i + 1].x, "n={}", n);
                assert!(l.dividers[i].vertical, "columns divide vertically");
            }
            assert_eq!(b[n - 1].right(), 1800, "n={}", n);
        }
    }

    #[test]
    fn every_pane_keeps_its_minimum_width() {
        // Four dividers crowded into the left edge. Placement is what enforces
        // the minimum now — a ratio is a fraction of whatever space its node
        // actually got, and only placement knows what that was.
        let m = Metrics::for_dpi(96);
        let mut tree = Node::columns(4);
        for i in 0..3 {
            tree.set_ratio(i, 0.01);
        }
        let l = build_tree(&tree, 1800, 800, &[]);
        for p in tree.leaves() {
            let b = l.pane(p).bounds;
            assert!(b.w >= m.min_pane_w, "pane too narrow: {:?}", b);
        }
    }

    #[test]
    fn a_window_too_narrow_to_divide_gives_the_space_to_one_pane() {
        // Two panes that cannot both meet the minimum is not two panes.
        // Placement stops dividing rather than producing a pair too narrow to
        // use, which is the same answer the old even-share clamp reached by a
        // longer route.
        let m = Metrics::for_dpi(96);
        let tree = Node::columns(4);
        let l = build_tree(&tree, 400, 600, &[]);
        let on_screen: Vec<Rect> = placed(&l, &tree)
            .into_iter()
            .filter(|b| !b.is_empty())
            .collect();
        assert_eq!(on_screen.len(), 1, "no room for a second");
        assert!(l.dividers.is_empty());
        // It gets the whole body. Not the minimum — a window narrower than one
        // pane's minimum cannot be made wider by refusing to draw in it.
        assert_eq!(on_screen[0].x, l.sidebar.right());
        assert_eq!(on_screen[0].right(), 400);
        let _ = m;
    }

    #[test]
    fn a_dropped_pane_leaves_no_divider_behind() {
        // Ctrl+1 after Ctrl+4. The tree is rebuilt, so there is nothing stale
        // left to leave a divider behind — which is what the fraction list
        // could do and why it had to be cleared by hand.
        let tree = Node::columns(1);
        let l = build_tree(&tree, 1800, 800, &[]);
        assert!(l.dividers.is_empty());
        assert_eq!(l.pane(PaneId(0)).bounds.right(), 1800);
        assert!(l.pane(PaneId(1)).bounds.is_empty(), "pane 1 is not in the tree");
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
            &Node::columns(1),
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
    fn a_divider_dragged_past_the_end_stops_at_the_minimum() {
        // Ratios are not clamped when they are set — they are a fraction of
        // whatever space the node actually got, and only placement knows what
        // that was. So placement is what keeps both sides usable.
        let m = Metrics::for_dpi(96);
        for (ratio, expect_left) in [(-5.0, true), (5.0, false)] {
            let mut tree = Node::columns(2);
            tree.set_ratio(0, ratio);
            let l = build_tree(&tree, 1000, 600, &[]);
            let a = l.pane(PaneId(0)).bounds;
            let b = l.pane(PaneId(1)).bounds;
            assert!(a.w >= m.min_pane_w, "left pane {:?}", a);
            assert!(b.w >= m.min_pane_w, "right pane {:?}", b);
            assert_eq!(a.right() + m.divider_w, b.x, "and they still meet");
            // Dragged hard left it is the left pane that sits at the minimum,
            // and hard right the other one.
            if expect_left {
                assert_eq!(a.w, m.min_pane_w);
            } else {
                assert_eq!(b.w, m.min_pane_w);
            }
        }
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
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "jamb", "src", "deep",
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
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "jamb", "src",
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
        let tree = Node::columns(2);
        let l = Layout::compute(
            client,
            m,
            &tree,
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
        let tree = Node::columns(2);
        let l = Layout::compute(
            client,
            m,
            &tree,
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
            &Node::columns(1),
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
        let tree = Node::columns(2);
        let mk = |offset: u32| {
            Layout::compute(
                client,
                m,
                &tree,
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
        let tree = Node::columns(2);
        let l = Layout::compute(
            client,
            m,
            &tree,
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
            assert_eq!(l.hit_test(d.rect.x + d.rect.w / 2, 400), Hit::Divider(i));
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
        let tree = Node::columns(2);
        let l = Layout::compute(
            client,
            m,
            &tree,
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
            &Node::columns(2),
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
                &Node::columns(1),
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
        assert_eq!(without.pane(PaneId(0)).bounds.right(), client.right());

        let with = mk(true);
        assert_eq!(with.inspector.w, m.inspector_w);
        assert_eq!(with.inspector.right(), client.right());
        assert_eq!(with.inspector.y, with.caption.bottom(), "clear of the caption");
        assert_eq!(
            with.inspector.bottom(),
            client.bottom(),
            "and everything below it, like the sidebar"
        );
        // The pane stops where the inspector starts: no overlap, no gap.
        assert_eq!(with.pane(PaneId(0)).bounds.right(), with.inspector.x);
        assert_eq!(
            with.pane(PaneId(0)).bounds.w,
            without.pane(PaneId(0)).bounds.w - m.inspector_w
        );
    }

    #[test]
    fn the_inspector_takes_its_width_out_of_the_panes_not_the_minimums() {
        let m = Metrics::for_dpi(96);
        // Wide enough for four panes, but not for four plus an inspector.
        let needed = m.min_pane_w * 4 + m.divider_w * 3;
        // Half an inspector of slack: enough for four panes on its own, and
        // short of it once the inspector takes its width.
        let client = Rect::new(0, 0, needed + m.inspector_w / 2, 800);
        let tabs = strs(&["Foo"]);
        let tree = Node::columns(4);
        let inputs: Vec<PaneInput> = (0..4).map(|_| input(&tabs, &[], 10, 0)).collect();
        let mk = |inspector: bool| {
            Layout::compute(client, m, &tree, false, &[], 0, inspector, &inputs, &FixedWidth)
        };

        let roomy = mk(false);
        assert_eq!(tree.leaves().len(), 4);
        for p in tree.leaves() {
            assert!(!roomy.pane(p).bounds.is_empty(), "all four fit without it");
        }

        // With the inspector there is no longer room for four that each meet
        // the minimum, so placement stops dividing rather than producing panes
        // too narrow to use.
        let squeezed = mk(true);
        let shown = tree
            .leaves()
            .iter()
            .filter(|p| !squeezed.pane(**p).bounds.is_empty())
            .count();
        assert!(shown < 4, "something had to give");
        for p in tree.leaves() {
            let b = squeezed.pane(p).bounds;
            assert!(b.is_empty() || b.w >= m.min_pane_w, "{:?}", b);
        }
    }

    #[test]
    fn a_tile_puts_its_name_beside_the_icon_and_list_fits_more_of_them() {
        let m = Metrics::for_dpi(96);
        let tiles = grid_pane_sized(1000, TILES);
        let tp = tiles.pane(PaneId(0));
        assert!(tp.side_label);
        let (icon, label) = tp.cell_parts(0, 0, m).unwrap();
        assert!(label.x >= icon.right(), "the name starts after the icon");
        assert!(
            label.y < icon.bottom() && label.bottom() > icon.y,
            "and on the same line as it"
        );

        // The same icon size with the label under it is the icon view.
        let under = grid_pane_sized(1000, -TILES);
        let up = under.pane(PaneId(0));
        assert!(!up.side_label);
        let (icon, label) = up.cell_parts(0, 0, m).unwrap();
        assert!(label.y >= icon.bottom(), "under it, not beside it");

        // List is the same shape at a smaller icon, so more cells fit a line.
        let list = grid_pane_sized(1000, LIST);
        assert!(list.pane(PaneId(0)).columns_per_line > tp.columns_per_line);

        // Stepping the wheel changes the size and leaves the side alone.
        assert!(beside(m.step_icons(TILES, true)));
        assert_eq!(m.step_icons(TILES, true).abs(), m.step_icons(-TILES, true));
    }

    fn grid_pane(w: i32) -> Layout {
        grid_pane_sized(w, DEFAULT_ICONS)
    }

    fn grid_pane_sized(w: i32, icons: i32) -> Layout {
        let tabs = strs(&["C:\\"]);
        let crumbs = strs(&["C:"]);
        Layout::compute(
            Rect::new(0, 0, w, 800),
            Metrics::for_dpi(96),
            &Node::columns(1),
            false,
            &[],
            0,
            false,
            &[PaneInput {
                tab_labels: &tabs,
                active_tab: 0,
                crumbs: &crumbs,
                total_lines: 50,
                scroll_offset: 0,
                icons,
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
        let (cell_w, cell_h, _) = m.cell(DEFAULT_ICONS);
        assert_eq!(p.columns_per_line, (p.list.w / cell_w).max(1) as u32);
        assert_eq!(p.line_h, cell_h);
        assert!(p.columns_per_line > 1, "1000px should fit several cells");

        // The first line's cells sit side by side, all at the same height.
        let cols = p.columns_per_line;
        let first = p.cell(0, 0).unwrap();
        let second = p.cell(1, 0).unwrap();
        assert_eq!(first.y, second.y);
        assert_eq!(first.right(), second.x);
        // The next line starts one cell height down.
        assert_eq!(p.cell(cols, 0).unwrap().y, first.y + cell_h);
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

    #[test]
    fn stepping_icon_sizes_walks_off_the_bottom_into_the_details_list() {
        let m = Metrics::for_dpi(96);
        // Up from the list enters at the default rather than the smallest, so
        // one notch produces something worth looking at.
        assert_eq!(m.step_icons(ICONS_OFF, true), DEFAULT_ICONS);
        // Down from the list stays there: there is nothing below it.
        assert_eq!(m.step_icons(ICONS_OFF, false), ICONS_OFF);

        // Down from the smallest icon is the list.
        assert_eq!(m.step_icons(ICON_STEPS[0], false), ICONS_OFF);
        // Up from the largest stays there rather than wrapping round to tiny.
        let biggest = *ICON_STEPS.last().unwrap();
        assert_eq!(m.step_icons(biggest, true), biggest);

        // And every step in between moves exactly one place.
        for pair in ICON_STEPS.windows(2) {
            assert_eq!(m.step_icons(pair[0], true), pair[1]);
            assert_eq!(m.step_icons(pair[1], false), pair[0]);
        }
    }

    #[test]
    fn a_bigger_icon_means_bigger_cells_and_fewer_of_them() {
        let m = Metrics::for_dpi(96);
        let small = grid_pane_sized(1000, 48);
        let large = grid_pane_sized(1000, 192);
        assert!(large.panes[0].line_h > small.panes[0].line_h);
        assert!(large.panes[0].columns_per_line < small.panes[0].columns_per_line);
        assert!(large.panes[0].cell_icon > small.panes[0].cell_icon);
        // The default size still lays out exactly as the fixed cell used to.
        let (w, h, icon) = m.cell(DEFAULT_ICONS);
        assert_eq!((w, h, icon), (118, 124, 72));
    }

    #[test]
    fn the_top_row_of_panes_puts_its_tabs_in_the_caption() {
        let m = Metrics::for_dpi(96);
        let tabs = strs(&["C:\\"]);
        let crumbs = strs(&["C:"]);
        let client = Rect::new(0, 0, 1400, 800);
        let mk = |sidebar: bool| {
            Layout::compute(
                client,
                m,
                &Node::columns(2),
                sidebar,
                &sidebar_of(2),
                0,
                false,
                &[
                    input(&tabs, &crumbs, 10, 0),
                    input(&tabs, &crumbs, 10, 0),
                ],
                &FixedWidth,
            )
        };

        let l = mk(true);
        // No row is taken off the top: the panes run up into the caption, and
        // their tab bars are what fills it.
        assert_eq!(l.panes[0].bounds.y, client.y);
        assert_eq!(l.panes[0].tab_bar.y, l.caption.y);
        assert_eq!(l.panes[0].tab_bar.h, l.caption.h);
        // The sidebar does not: the logo and the toggle live above it.
        assert_eq!(l.sidebar.y, l.caption.bottom());
        assert!(l.caption_sidebar.right() <= l.sidebar.right());

        // The window buttons are the right end of the caption, and the pane
        // under them stops short rather than drawing a tab behind one.
        assert_eq!(l.win_close.right(), client.right());
        assert!(l.panes[1].tab_bar.right() <= l.win_min.x);
        // The pane itself still reaches the edge — it is only the strip that
        // keeps clear.
        assert_eq!(l.panes[1].bounds.right(), client.right());

        // With no sidebar the first pane owns the left end, so its strip is
        // the one that has to make room.
        let l = mk(false);
        assert_eq!(l.panes[0].bounds.x, client.x);
        assert!(l.panes[0].tab_bar.x >= l.caption_sidebar.right());
    }

    #[test]
    fn a_row_never_ends_up_smaller_than_its_text() {
        let base = Metrics::for_dpi(96);
        let big = Metrics::sized(96, 150, 100);
        assert!(big.row_h > base.row_h, "text grew, the row did not");

        // Density thins a row out, but only on top of what the text needs:
        // the most compact setting at 150% text is still roomier than normal
        // at 100%, because the alternative is clipped glyphs.
        let compact = Metrics::sized(96, 150, DENSITY_MIN);
        assert!(compact.row_h > base.row_h, "density undercut the font");
        assert!(compact.row_h < big.row_h, "and still did something");

        // A scrollbar is a pointer target, not a reading distance.
        assert_eq!(big.scrollbar_w, base.scrollbar_w);

        // A drive row is two lines of text and a capacity bar with no slack
        // between them, so density does not get to squeeze it: at compact, the
        // bar was being drawn through the label.
        assert_eq!(
            Metrics::sized(96, 100, DENSITY_MIN).sidebar_drive_h,
            base.sidebar_drive_h
        );
        assert!(Metrics::sized(96, 150, DENSITY_MIN).sidebar_drive_h > base.sidebar_drive_h);

        // A settings file is text, and text gets typed.
        assert_eq!(
            Metrics::sized(96, 10_000, -5).row_h,
            Metrics::sized(96, FONT_MAX, DENSITY_MIN).row_h
        );
    }

    #[test]
    fn too_many_tabs_scroll_the_strip_rather_than_shrink_to_slivers() {
        let m = Metrics::for_dpi(96);
        let labels: Vec<String> = (0..40).map(|i| format!("Tab {i}")).collect();
        let bar = Rect::new(0, 0, 600, m.tab_bar_h);
        let floor = m.tab_min_w * 3 / 4;

        let first = tab_rects(bar, m, &labels, 0, &FixedWidth);
        assert_eq!(first.len(), labels.len(), "one entry per tab, always");
        let shown: Vec<&TabRect> = first.iter().filter(|t| !t.full.is_empty()).collect();
        assert!(shown.len() < labels.len(), "they cannot all fit");
        for t in &shown {
            assert!(t.full.w >= floor, "no tab shrinks below readable");
            assert!(t.full.right() <= bar.right(), "and none runs off the end");
        }
        assert!(!first[0].full.is_empty(), "the active tab is on screen");
        assert!(first[39].full.is_empty(), "the far end is not");

        // Switching to a tab off the end brings it into view, which is the
        // only way back to it.
        let last = tab_rects(bar, m, &labels, 39, &FixedWidth);
        assert!(!last[39].full.is_empty());
        assert!(last[0].full.is_empty());
    }

    #[test]
    fn both_side_panels_have_an_edge_to_pull() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["C:"]);
        let l = Layout::compute(
            client, m, &Node::columns(1), true, &sidebar_of(2), 0, true,
            &[input(&tabs, &tabs, 10, 0)], &FixedWidth,
        );
        // Inside its own panel, not across the line: the pane's Back button
        // starts at the very first pixel the pane owns.
        assert_eq!(l.sidebar_edge.right(), l.sidebar.right());
        assert_eq!(l.inspector_edge.x, l.inspector.x);
        assert_eq!(
            l.hit_test(l.sidebar.right() - 1, l.sidebar.y + 40),
            Hit::PanelEdge { sidebar: true }
        );
        assert_eq!(
            l.hit_test(l.inspector.x + 1, l.inspector.y + 40),
            Hit::PanelEdge { sidebar: false }
        );
        let back = l.pane(PaneId(0)).nav_back;
        assert_eq!(
            l.hit_test(back.x + 1, back.y + 1),
            Hit::Nav(PaneId(0), NavButton::Back)
        );
    }

    #[test]
    fn every_pane_carries_its_own_overflow_menu() {
        let l = build_n(2, 1400, 800, &strs(&["C:"]));
        for pid in [PaneId(0), PaneId(1)] {
            let p = l.pane(pid);
            assert_eq!(p.overflow.right(), p.toolbar.right(), "against the far end");
            assert!(p.breadcrumb.right() <= p.overflow.x, "clear of the crumbs");
            let (x, y) = (p.overflow.x + 1, p.overflow.y + 1);
            assert_eq!(l.hit_test(x, y), Hit::Overflow(pid));
        }
    }

    #[test]
    fn a_tab_dropped_in_the_middle_goes_in_and_near_an_edge_splits() {
        let l = build_n(1, 1000, 800, &[]);
        let r = l.pane(PaneId(0)).bounds;
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        assert_eq!(l.drop_zone(cx, cy).unwrap().1, DropZone::Into);

        // Each edge, and the half it lights up is the half that edge would
        // become \u2014 the highlight is a promise about the drop.
        let (_, z, half) = l.drop_zone(r.x + 2, cy).unwrap();
        assert_eq!(z, DropZone::Left);
        assert_eq!((half.x, half.w), (r.x, r.w / 2));

        let (_, z, half) = l.drop_zone(r.right() - 2, cy).unwrap();
        assert_eq!(z, DropZone::Right);
        assert_eq!(half.right(), r.right());

        // Below the tab bar, because the bar itself never splits: a click on
        // a tab that twitches five pixels is a reorder, not a new pane.
        let bar = l.pane(PaneId(0)).tab_bar;
        let (_, z, half) = l.drop_zone(cx, bar.bottom() + 2).unwrap();
        assert_eq!(z, DropZone::Top);
        assert_eq!((half.y, half.h), (r.y, r.h / 2));

        assert_eq!(
            l.drop_zone(cx, bar.y + bar.h / 2).unwrap().1,
            DropZone::Into,
            "a drop on the tab bar moves or reorders"
        );

        let (_, z, half) = l.drop_zone(cx, r.bottom() - 2).unwrap();
        assert_eq!(z, DropZone::Bottom);
        assert_eq!(half.bottom(), r.bottom());

        // Nothing is under a point outside every pane.
        assert!(l.drop_zone(-10, -10).is_none());
    }

    #[test]
    fn a_corner_belongs_to_the_edge_it_is_nearer_in_proportion() {
        // A short wide pane. Forty pixels from the left is a twentieth of the
        // way in; thirty from the top is a fifth. Fewer pixels is not nearer
        // when the pane is not square, and aiming is done by eye.
        let l = build_n(1, 1000, 200, &[]);
        let r = l.pane(PaneId(0)).bounds;
        let (_, zone, _) = l.drop_zone(r.x + 40, r.y + 30).unwrap();
        assert_eq!(zone, DropZone::Left);
    }

    #[test]
    fn a_zone_names_the_split_it_makes() {
        // Left and Top put the new pane first, which is what `before` means,
        // and Into is not a split at all.
        assert_eq!(DropZone::Into.split(), None);
        assert_eq!(DropZone::Left.split(), Some((true, true)));
        assert_eq!(DropZone::Right.split(), Some((true, false)));
        assert_eq!(DropZone::Top.split(), Some((false, true)));
        assert_eq!(DropZone::Bottom.split(), Some((false, false)));

        // And the tree puts it where the zone said.
        let mut tree = Node::columns(1);
        assert!(tree.split_at(PaneId(0), PaneId(1), true, true));
        assert_eq!(tree.leaves(), vec![PaneId(1), PaneId(0)], "new pane on the left");
        let l = build_tree(&tree, 1000, 600, &[]);
        assert!(l.pane(PaneId(1)).bounds.x < l.pane(PaneId(0)).bounds.x);
    }

    #[test]
    fn splitting_a_leaf_keeps_everything_else_where_it_was() {
        let mut tree = Node::columns(2);
        assert!(tree.split_at(PaneId(1), PaneId(2), false, false));
        assert_eq!(tree.leaves(), vec![PaneId(0), PaneId(1), PaneId(2)]);

        // The new pane is stacked under the one that was split, and the first
        // column is untouched: that is the whole point of a tree.
        let l = build_tree(&tree, 1800, 800, &[]);
        let a = l.pane(PaneId(0)).bounds;
        let b = l.pane(PaneId(1)).bounds;
        let c = l.pane(PaneId(2)).bounds;
        assert_eq!(a.h, 800, "the untouched column is still full height");
        assert_eq!(b.x, c.x, "the split pair share a column");
        assert_eq!(b.w, c.w);
        assert!(b.bottom() < c.y, "and one sits above the other");
        assert_eq!(l.dividers.len(), 2);
        assert!(l.dividers[0].vertical);
        assert!(!l.dividers[1].vertical, "the new one divides horizontally");
    }

    #[test]
    fn closing_a_pane_collapses_the_split_that_held_it() {
        let mut tree = Node::columns(3);
        assert!(tree.close(PaneId(1)));
        assert_eq!(tree.leaves(), vec![PaneId(0), PaneId(2)]);
        // The survivor takes the whole of what the pair had, rather than
        // leaving a gap where its sibling was.
        let l = build_tree(&tree, 1800, 800, &[]);
        assert_eq!(l.dividers.len(), 1);
        assert_eq!(
            l.pane(PaneId(2)).bounds.right(),
            1800,
            "no gap left behind"
        );

        // The last pane cannot be closed: a window with no panes is not a view.
        let mut one = Node::columns(1);
        assert!(!one.close(PaneId(0)));
        assert_eq!(one.count(), 1);
    }

    #[test]
    fn a_new_pane_takes_an_id_nobody_is_using() {
        let mut tree = Node::columns(2);
        assert_eq!(tree.unused(), vec![PaneId(2), PaneId(3)]);
        tree.split_at(PaneId(0), PaneId(2), true, false);
        assert_eq!(tree.unused(), vec![PaneId(3)]);
        tree.split_at(PaneId(3), PaneId(1), true, false);
        assert!(tree.unused().contains(&PaneId(3)), "no such leaf, no split");
    }

    #[test]
    fn set_ratio_addresses_dividers_in_the_order_they_are_drawn() {
        // The drag handler knows a divider by its index in `dividers`, so the
        // same index has to reach the same split node.
        let mut tree = Node::columns(2);
        tree.split_at(PaneId(1), PaneId(2), false, false);
        for (i, ratio) in [(0usize, 0.25f32), (1, 0.75)] {
            let mut t = tree.clone();
            t.set_ratio(i, ratio);
            let l = build_tree(&t, 1800, 800, &[]);
            let before = build_tree(&tree, 1800, 800, &[]);
            let moved: Vec<bool> = (0..l.dividers.len())
                .map(|d| l.dividers[d].rect != before.dividers[d].rect)
                .collect();
            assert!(moved[i], "divider {} should have moved", i);
        }
    }
}
