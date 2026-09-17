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
        self.w > 0 && self.h > 0 && px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    /// Shrink on all sides.
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
    pub breadcrumb_h: i32,
    pub header_h: i32,
    pub status_h: i32,
    pub row_h: i32,
    pub sidebar_w: i32,
    /// Band above the drive rows holding the "DRIVES" caption.
    pub sidebar_header_h: i32,
    pub divider_w: i32,
    pub scrollbar_w: i32,
    pub scrollbar_min_thumb: i32,
    pub min_pane_w: i32,
    pub icon_size: i32,
    pub pad: i32,
    pub col_size_w: i32,
    pub col_date_w: i32,
    pub col_min_name_w: i32,
    pub crumb_sep_w: i32,
}

impl Metrics {
    pub fn for_dpi(dpi: u32) -> Self {
        let scale = dpi as f32 / 96.0;
        let s = |v: f32| (v * scale).round() as i32;
        Self {
            scale,
            tab_bar_h: s(30.0),
            tab_min_w: s(90.0),
            tab_max_w: s(200.0),
            tab_close_w: s(20.0),
            new_tab_w: s(28.0),
            breadcrumb_h: s(28.0),
            header_h: s(26.0),
            status_h: s(24.0),
            row_h: s(24.0),
            sidebar_w: s(180.0),
            sidebar_header_h: s(26.0),
            divider_w: s(6.0),
            scrollbar_w: s(12.0),
            scrollbar_min_thumb: s(28.0),
            min_pane_w: s(220.0),
            icon_size: s(16.0),
            pad: s(8.0),
            col_size_w: s(90.0),
            col_date_w: s(130.0),
            col_min_name_w: s(120.0),
            crumb_sep_w: s(14.0),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::for_dpi(96)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
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
}

#[derive(Clone, Debug, Default)]
pub struct PaneLayout {
    pub bounds: Rect,
    pub tab_bar: Rect,
    pub tabs: Vec<TabRect>,
    pub new_tab: Rect,
    pub breadcrumb: Rect,
    pub crumbs: Vec<CrumbRect>,
    pub header: Rect,
    pub columns: Vec<ColumnRect>,
    /// The rows area, excluding the scrollbar gutter.
    pub list: Rect,
    pub scrollbar: Rect,
    pub thumb: Rect,
    /// Rows fully visible in `list`.
    pub visible_rows: u32,
}

impl PaneLayout {
    /// Absolute row index under a point, if the point is over a real row.
    pub fn row_at(&self, py: i32, scroll_offset: u32, row_h: i32, total_rows: u32) -> Option<u32> {
        if row_h <= 0 || !self.list.contains(self.list.x, py) {
            return None;
        }
        let rel = py - self.list.y;
        if rel < 0 {
            return None;
        }
        let row = scroll_offset as i64 + (rel / row_h) as i64;
        if row >= 0 && (row as u64) < total_rows as u64 {
            Some(row as u32)
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
    pub drives: Vec<Rect>,
    pub left: PaneLayout,
    pub right: PaneLayout,
    pub divider: Rect,
    pub status: Rect,
}

/// What is under the cursor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Nothing,
    Drive(usize),
    Divider,
    Tab(Side, usize),
    TabClose(Side, usize),
    NewTab(Side),
    Crumb(Side, usize),
    Column(Side, SortKey),
    /// Absolute row index within the pane's list.
    Row(Side, u32),
    ListBackground(Side),
    ScrollbarThumb(Side),
    /// Click on the track above or below the thumb; the f32 is 0..1 along the track.
    ScrollbarTrack(Side, f32),
}

/// Everything the layout needs to know about one pane to size it.
pub struct PaneInput<'a> {
    pub tab_labels: &'a [String],
    pub crumbs: &'a [String],
    pub total_rows: u32,
    pub scroll_offset: u32,
}

impl Layout {
    /// Compute the whole frame. `split_x` is the divider's left edge, already
    /// clamped by `clamp_split`.
    pub fn compute(
        client: Rect,
        metrics: Metrics,
        split_x: i32,
        sidebar_visible: bool,
        drive_count: usize,
        left: &PaneInput,
        right: &PaneInput,
        measurer: &dyn TextMeasurer,
    ) -> Layout {
        let m = metrics;
        let (body, status) = client.split_bottom(m.status_h);

        let (sidebar, body) = if sidebar_visible {
            body.split_left(m.sidebar_w)
        } else {
            (Rect::default(), body)
        };

        // Drive rows start below the caption band so the two never overlap.
        let drives = (0..drive_count)
            .map(|i| {
                Rect::new(
                    sidebar.x,
                    sidebar.y + m.sidebar_header_h + i as i32 * m.row_h,
                    sidebar.w,
                    m.row_h,
                )
            })
            .filter(|r| r.bottom() <= sidebar.bottom())
            .collect();

        // split_x is measured from the left edge of the pane area.
        let split_local = split_x - body.x;
        let left_w = split_local.clamp(0, body.w.max(0));
        let left_bounds = Rect::new(body.x, body.y, left_w, body.h);
        let divider = Rect::new(body.x + left_w, body.y, m.divider_w, body.h);
        let right_bounds = Rect::new(
            divider.right(),
            body.y,
            (body.right() - divider.right()).max(0),
            body.h,
        );

        Layout {
            metrics: m,
            client,
            sidebar,
            drives,
            left: pane_layout(left_bounds, m, left, measurer),
            right: pane_layout(right_bounds, m, right, measurer),
            divider,
            status,
        }
    }

    /// Clamp a proposed divider position so both panes keep a usable width.
    pub fn clamp_split(client: Rect, metrics: Metrics, sidebar_visible: bool, split_x: i32) -> i32 {
        let m = metrics;
        let body_left = client.x + if sidebar_visible { m.sidebar_w } else { 0 };
        let body_right = client.right();
        let min = body_left + m.min_pane_w;
        let max = body_right - m.min_pane_w - m.divider_w;
        if max < min {
            // Window too narrow to honour both minimums: split down the middle.
            return body_left + (body_right - body_left) / 2;
        }
        split_x.clamp(min, max)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        if self.divider.contains(x, y) {
            return Hit::Divider;
        }
        for (i, r) in self.drives.iter().enumerate() {
            if r.contains(x, y) {
                return Hit::Drive(i);
            }
        }
        if let Some(h) = hit_pane(&self.left, Side::Left, x, y) {
            return h;
        }
        if let Some(h) = hit_pane(&self.right, Side::Right, x, y) {
            return h;
        }
        Hit::Nothing
    }

    pub fn pane(&self, side: Side) -> &PaneLayout {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }
}

fn hit_pane(p: &PaneLayout, side: Side, x: i32, y: i32) -> Option<Hit> {
    if !p.bounds.contains(x, y) {
        return None;
    }
    if p.tab_bar.contains(x, y) {
        if p.new_tab.contains(x, y) {
            return Some(Hit::NewTab(side));
        }
        for (i, t) in p.tabs.iter().enumerate() {
            if t.close.contains(x, y) {
                return Some(Hit::TabClose(side, i));
            }
            if t.full.contains(x, y) {
                return Some(Hit::Tab(side, i));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.breadcrumb.contains(x, y) {
        for c in &p.crumbs {
            if c.rect.contains(x, y) {
                return Some(Hit::Crumb(side, c.segment_index));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.header.contains(x, y) {
        for c in &p.columns {
            if c.rect.contains(x, y) {
                return Some(Hit::Column(side, c.key));
            }
        }
        return Some(Hit::Nothing);
    }
    if p.scrollbar.contains(x, y) {
        if p.thumb.contains(x, y) {
            return Some(Hit::ScrollbarThumb(side));
        }
        let t = if p.scrollbar.h > 0 {
            ((y - p.scrollbar.y) as f32 / p.scrollbar.h as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return Some(Hit::ScrollbarTrack(side, t));
    }
    if p.list.contains(x, y) {
        return Some(Hit::ListBackground(side));
    }
    Some(Hit::Nothing)
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
    let (breadcrumb, rest) = rest.split_top(m.breadcrumb_h);
    let (header, list_area) = rest.split_top(m.header_h);
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

    let crumbs = crumb_rects(breadcrumb, m, input.crumbs, measurer);
    let columns = column_rects(header, m);

    let visible_rows = if m.row_h > 0 {
        (list.h / m.row_h).max(0) as u32
    } else {
        0
    };
    let thumb = thumb_rect(scrollbar, m, input.total_rows, visible_rows, input.scroll_offset);

    PaneLayout {
        bounds,
        tab_bar,
        tabs,
        new_tab,
        breadcrumb,
        crumbs,
        header,
        columns,
        list,
        scrollbar,
        thumb,
        visible_rows,
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
            (text + m.pad * 2 + m.tab_close_w).clamp(m.tab_min_w, m.tab_max_w)
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
        // Only offer a close target if there is room for one that is not
        // covering the whole tab.
        let close = if full.w > m.tab_close_w * 2 {
            Rect::new(
                full.right() - m.tab_close_w,
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
        if i > first || first > 0 {
            // Separator already accounted for above.
        }
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
    // Size and Date get fixed widths; Name absorbs the rest. When the pane is
    // narrow the trailing columns drop off rather than squeezing Name to nothing.
    let mut cols = Vec::new();
    let mut name_w = header.w;
    let mut show_date = false;
    let mut show_size = false;

    if header.w - m.col_size_w >= m.col_min_name_w {
        show_size = true;
        name_w -= m.col_size_w;
    }
    if show_size && header.w - m.col_size_w - m.col_date_w >= m.col_min_name_w {
        show_date = true;
        name_w -= m.col_date_w;
    }

    cols.push(ColumnRect {
        key: SortKey::Name,
        rect: Rect::new(header.x, header.y, name_w, header.h),
    });
    let mut x = header.x + name_w;
    if show_size {
        cols.push(ColumnRect {
            key: SortKey::Size,
            rect: Rect::new(x, header.y, m.col_size_w, header.h),
        });
        x += m.col_size_w;
    }
    if show_date {
        cols.push(ColumnRect {
            key: SortKey::Date,
            rect: Rect::new(x, header.y, m.col_date_w, header.h),
        });
    }
    cols
}

fn thumb_rect(track: Rect, m: Metrics, total: u32, visible: u32, offset: u32) -> Rect {
    if track.h <= 0 || total == 0 || visible >= total {
        return Rect::default();
    }
    let frac = visible as f32 / total as f32;
    let h = ((track.h as f32 * frac) as i32).max(m.scrollbar_min_thumb).min(track.h);
    let max_off = total.saturating_sub(visible).max(1);
    let t = (offset as f32 / max_off as f32).clamp(0.0, 1.0);
    let y = track.y + ((track.h - h) as f32 * t) as i32;
    Rect::new(track.x, y, track.w, h)
}

/// Invert `thumb_rect`: where should scroll_offset land for a drag to `py`?
pub fn scroll_offset_for_thumb_y(
    track: Rect,
    m: Metrics,
    total: u32,
    visible: u32,
    py: i32,
) -> u32 {
    if total == 0 || visible >= total || track.h <= 0 {
        return 0;
    }
    let frac = visible as f32 / total as f32;
    let h = ((track.h as f32 * frac) as i32).max(m.scrollbar_min_thumb).min(track.h);
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
            total_rows: rows,
            scroll_offset: scroll,
        }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn build(w: i32, h: i32, crumbs: &[String]) -> Layout {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, w, h);
        let tabs = strs(&["Foo"]);
        let split = Layout::clamp_split(client, m, true, w / 2);
        Layout::compute(
            client,
            m,
            split,
            true,
            3,
            &input(&tabs, crumbs, 100, 0),
            &input(&tabs, crumbs, 100, 0),
            &FixedWidth,
        )
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
        let l = build(1400, 800, &strs(&["C:\\", "Users"]));
        assert_eq!(l.left.bounds.right(), l.divider.x);
        assert_eq!(l.divider.right(), l.right.bounds.x);
        assert_eq!(l.right.bounds.right(), l.client.right());
        assert_eq!(l.left.bounds.x, l.sidebar.right());
    }

    #[test]
    fn clamp_split_keeps_both_panes_usable() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1000, 600);
        assert_eq!(Layout::clamp_split(client, m, false, -500), m.min_pane_w);
        assert_eq!(
            Layout::clamp_split(client, m, false, 5000),
            1000 - m.min_pane_w - m.divider_w
        );
    }

    #[test]
    fn clamp_split_centres_when_the_window_is_too_narrow() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 200, 600);
        let s = Layout::clamp_split(client, m, false, 10);
        assert_eq!(s, 100);
    }

    // -- the bug this module was written to kill ---------------------------

    #[test]
    fn crumb_hit_test_matches_where_the_crumb_is_drawn() {
        let crumbs = strs(&["C:\\", "Users", "Sindroma", "Desktop"]);
        let l = build(1600, 800, &crumbs);
        assert_eq!(l.left.crumbs.len(), 4, "all four should fit at this width");

        // Click the centre of each drawn crumb; it must resolve to that crumb.
        for c in &l.left.crumbs {
            let cx = c.rect.x + c.rect.w / 2;
            let cy = c.rect.y + c.rect.h / 2;
            assert_eq!(
                l.hit_test(cx, cy),
                Hit::Crumb(Side::Left, c.segment_index),
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
        let widths: Vec<i32> = l.left.crumbs.iter().map(|c| c.rect.w).collect();
        assert!(
            widths[2] > widths[1] * 3,
            "long crumb must be much wider than a short one, got {:?}",
            widths
        );
    }

    #[test]
    fn crumbs_never_escape_the_breadcrumb_bar() {
        // The old renderer walked a fixed stride and spilled into the next pane.
        let crumbs = strs(&[
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "file-xplorer", "src", "deep",
        ]);
        let l = build(900, 800, &crumbs);
        for c in &l.left.crumbs {
            assert!(
                c.rect.right() <= l.left.breadcrumb.right(),
                "crumb {:?} spilled past {:?}",
                c.rect,
                l.left.breadcrumb
            );
        }
    }

    #[test]
    fn overflowing_crumbs_elide_from_the_left_and_keep_the_leaf() {
        let crumbs = strs(&[
            "C:\\", "Users", "Sindroma", "Documents", "Projects", "file-xplorer", "src",
        ]);
        let l = build(760, 800, &crumbs);
        let last = l.left.crumbs.last().unwrap();
        assert_eq!(
            last.segment_index,
            crumbs.len() - 1,
            "the current folder must always be shown"
        );
        assert!(l.left.crumbs[0].is_ellipsis, "hidden parents get an ellipsis");
    }

    #[test]
    fn tab_close_button_is_inside_its_tab_and_hit_tests_first() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Documents", "Downloads"]);
        let crumbs = strs(&["C:\\"]);
        let l = Layout::compute(
            client,
            m,
            Layout::clamp_split(client, m, false, 700),
            false,
            0,
            &input(&tabs, &crumbs, 0, 0),
            &input(&tabs, &crumbs, 0, 0),
            &FixedWidth,
        );
        let t = l.left.tabs[1];
        assert!(t.close.right() <= t.full.right());
        let cx = t.close.x + t.close.w / 2;
        let cy = t.close.y + t.close.h / 2;
        assert_eq!(l.hit_test(cx, cy), Hit::TabClose(Side::Left, 1));
        // The body of the same tab still selects it.
        assert_eq!(l.hit_test(t.full.x + 2, t.full.y + 2), Hit::Tab(Side::Left, 1));
    }

    #[test]
    fn many_tabs_shrink_to_fit_instead_of_overflowing() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1000, 800);
        let tabs: Vec<String> = (0..12).map(|i| format!("Folder{}", i)).collect();
        let crumbs = strs(&["C:\\"]);
        let l = Layout::compute(
            client,
            m,
            Layout::clamp_split(client, m, false, 500),
            false,
            0,
            &input(&tabs, &crumbs, 0, 0),
            &input(&tabs, &crumbs, 0, 0),
            &FixedWidth,
        );
        for t in &l.left.tabs {
            assert!(
                t.full.right() <= l.left.tab_bar.right() + 1,
                "tab {:?} overflowed the bar {:?}",
                t.full,
                l.left.tab_bar
            );
        }
    }

    #[test]
    fn columns_drop_off_before_squeezing_the_name_column() {
        let wide = build(1600, 800, &strs(&["C:\\"]));
        assert_eq!(wide.left.columns.len(), 3);

        let narrow = build(700, 800, &strs(&["C:\\"]));
        assert!(
            narrow.left.columns.len() < 3,
            "narrow pane should shed columns, got {:?}",
            narrow.left.columns.len()
        );
        assert_eq!(narrow.left.columns[0].key, SortKey::Name);
    }

    #[test]
    fn column_headers_tile_the_header_exactly() {
        let l = build(1600, 800, &strs(&["C:\\"]));
        let cols = &l.left.columns;
        assert_eq!(cols[0].rect.x, l.left.header.x);
        for pair in cols.windows(2) {
            assert_eq!(pair[0].rect.right(), pair[1].rect.x);
        }
        assert_eq!(cols.last().unwrap().rect.right(), l.left.header.right());
    }

    #[test]
    fn row_hit_test_accounts_for_scroll() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let m = l.metrics;
        let p = &l.left;
        // Third visible row while scrolled down 10.
        let y = p.list.y + m.row_h * 2 + 1;
        assert_eq!(p.row_at(y, 10, m.row_h, 100), Some(12));
        // Past the end of the data.
        assert_eq!(p.row_at(y, 10, m.row_h, 5), None);
    }

    #[test]
    fn scrollbar_thumb_reflects_position_and_inverts_cleanly() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 1400, 800);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let mk = |offset: u32| {
            Layout::compute(
                client,
                m,
                Layout::clamp_split(client, m, false, 700),
                false,
                0,
                &input(&tabs, &crumbs, 1000, offset),
                &input(&tabs, &crumbs, 1000, offset),
                &FixedWidth,
            )
        };
        let top = mk(0);
        assert_eq!(top.left.thumb.y, top.left.scrollbar.y);

        let visible = top.left.visible_rows;
        let bottom = mk(1000 - visible);
        assert_eq!(
            bottom.left.thumb.bottom(),
            bottom.left.scrollbar.bottom(),
            "thumb should sit flush at the end of the track"
        );

        // Dragging the thumb back to the top yields offset 0.
        let track = top.left.scrollbar;
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
        let l = Layout::compute(
            client,
            m,
            Layout::clamp_split(client, m, false, 700),
            false,
            0,
            &input(&tabs, &crumbs, 3, 0),
            &input(&tabs, &crumbs, 3, 0),
            &FixedWidth,
        );
        assert!(l.left.thumb.is_empty());
    }

    #[test]
    fn divider_hit_wins_over_the_panes() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let cx = l.divider.x + l.divider.w / 2;
        assert_eq!(l.hit_test(cx, 400), Hit::Divider);
    }

    #[test]
    fn drive_rows_hit_test() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        assert_eq!(l.drives.len(), 3);
        let r = l.drives[1];
        assert_eq!(l.hit_test(r.x + 5, r.y + 2), Hit::Drive(1));
    }

    #[test]
    fn metrics_scale_with_dpi() {
        let lo = Metrics::for_dpi(96);
        let hi = Metrics::for_dpi(192);
        assert_eq!(hi.row_h, lo.row_h * 2);
        assert_eq!(hi.sidebar_w, lo.sidebar_w * 2);
    }

    #[test]
    fn zero_sized_pane_does_not_panic() {
        let m = Metrics::for_dpi(96);
        let client = Rect::new(0, 0, 0, 0);
        let tabs = strs(&["Foo"]);
        let crumbs = strs(&["C:\\"]);
        let l = Layout::compute(
            client,
            m,
            0,
            true,
            2,
            &input(&tabs, &crumbs, 0, 0),
            &input(&tabs, &crumbs, 0, 0),
            &FixedWidth,
        );
        assert_eq!(l.hit_test(0, 0), Hit::Nothing);
    }


    #[test]
    fn drive_rows_start_below_the_sidebar_caption() {
        let l = build(1400, 800, &strs(&["C:\\"]));
        let caption_bottom = l.sidebar.y + l.metrics.sidebar_header_h;
        assert!(
            l.drives[0].y >= caption_bottom,
            "first drive row at {} overlaps the caption band ending at {}",
            l.drives[0].y,
            caption_bottom
        );
    }
}
