// A searchable menu, drawn in the app's own colours.
//
// One box with a query field and a list under it: the context menu, the
// command palette, the font picker, the folder picker. Typing narrows the
// list; Enter takes what is highlighted.
//
// It replaced a stock LISTBOX, which brought keyboard navigation and scrolling
// for free and drew in the *system* theme — a white popup hanging off a dark
// window. Rows are painted here instead, which is also what lets a row carry
// an icon, a chevron into a submenu, or a handler's own little bitmap.
//
// GDI rather than Direct2D: the renderer's target belongs to the main window,
// and this is a window of its own. `menu.rs` paints the same way for the same
// reason.

use std::cell::RefCell;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, SetActiveWindow, SetCapture, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::icons::IconCache;
use crate::theme::{colorref, Palette, Rgb, Theme};

const CLASS: &str = "FileXplorerPicker";
const IDC_QUERY: i32 = 2001;
/// EM_SETCUEBANNER: the grey prompt an empty edit box shows.
const EM_SETCUEBANNER: u32 = 0x1501;

/// What separates a group from what is inside it, in a label.
///
/// `shellmenu::list` writes submenu paths this way and the picker reads them
/// back, so "7-Zip › Extract here" is one item and also a group with an item
/// in it. Nothing else has to agree on a tree.
pub const GROUP_SEP: &str = " \u{203a} ";

/// Segoe MDL2 code points used by the picker itself.
const GLYPH_SEARCH: &str = "\u{E721}";
const GLYPH_CHEVRON: &str = "\u{E76C}";
const GLYPH_BACK: &str = "\u{E72B}";

use crate::fs::wide;


pub struct Item {
    pub label: String,
    /// Shortcut or hint, shown in a chip at the right-hand end.
    pub detail: String,
    /// Icon cache key, as `icons::icon_key` builds them.
    pub icon: Option<String>,
    /// A bitmap to draw instead, for rows whose picture belongs to somebody
    /// else — a shell handler's own icon. Borrowed: whatever owns it must
    /// outlive the picker.
    pub bitmap: Option<HBITMAP>,
}

/// A row on screen: an item, a group to go into, or the way back out.
enum Row {
    Item(usize),
    /// Name and how many items are inside.
    Group(String, usize),
    Back,
}

/// Score `label` against `query`, higher is better. None means no match.
///
/// Subsequence matching, so "nf" finds "New folder". Consecutive characters and
/// word starts score higher, which is what makes short queries land on the
/// command people meant rather than the first alphabetical hit.
pub fn score(label: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().chars().collect();

    // A whole-query prefix match is the strongest signal there is, and must
    // outrank a scattered match that happens to land on more word starts:
    // typing "ref" means Refresh, not "Rename file".
    let prefix_bonus = if l.len() >= q.len() && l[..q.len()] == q[..] {
        20
    } else {
        0
    };

    let mut total = 0i32;
    let mut li = 0usize;
    let mut last_hit: Option<usize> = None;

    for &qc in &q {
        let mut found = None;
        while li < l.len() {
            if l[li] == qc {
                found = Some(li);
                break;
            }
            li += 1;
        }
        let hit = found?;
        total += 1;
        // Start of a word, or the very beginning.
        if hit == 0 || l[hit - 1] == ' ' || l[hit - 1] == '-' {
            total += 8;
        }
        if last_hit == Some(hit.wrapping_sub(1)) {
            total += 5; // consecutive
        }
        last_hit = Some(hit);
        li += 1;
    }
    // Prefer shorter labels when scores tie: less to read, likelier the target.
    Some(total + prefix_bonus - (label.chars().count() as i32) / 8)
}

/// Indices of `items` matching `query`, best first.
pub fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| score(&it.label, query).map(|s| (s, i)))
        .collect();
    // Stable by score then original order, so an empty query keeps the
    // author's grouping instead of scrambling it.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Where `query` appears in `label` as a run, for highlighting.
///
/// The run only, not every matched character: a scattered subsequence lit up
/// one letter at a time reads as noise, and the run is the case people are
/// looking at when they squint at a filtered list.
pub fn highlight(label: &str, query: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.len() > l.len() {
        return None;
    }
    (0..=l.len() - q.len())
        .find(|&i| l[i..i + q.len()] == q[..])
        .map(|i| (i, i + q.len()))
}

/// One level of a menu built from flattened labels.
///
/// Items below `path` that go no deeper are rows; everything deeper collapses
/// into the group it belongs to, in the order the groups first appear. Pure,
/// because this is the part that is easy to get wrong.
fn level(labels: &[String], path: &[String]) -> (Vec<usize>, Vec<(String, usize)>) {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{}{}", path.join(GROUP_SEP), GROUP_SEP)
    };
    let mut leaves = Vec::new();
    let mut groups: Vec<(String, usize)> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let Some(rest) = label.strip_prefix(prefix.as_str()) else {
            continue;
        };
        match rest.split_once(GROUP_SEP) {
            Some((head, _)) => match groups.iter_mut().find(|(n, _)| n == head) {
                Some((_, n)) => *n += 1,
                None => groups.push((head.to_string(), 1)),
            },
            None => leaves.push(i),
        }
    }
    (leaves, groups)
}

struct Metrics {
    row_h: i32,
    pad: i32,
    icon: i32,
    query_h: i32,
    radius: i32,
}

impl Metrics {
    fn new(dpi: u32) -> Metrics {
        let s = |v: f32| (v * dpi as f32 / 96.0).round() as i32;
        Metrics {
            row_h: s(24.0),
            pad: s(8.0),
            icon: s(16.0),
            query_h: s(32.0),
            radius: s(6.0),
        }
    }
}

struct State {
    items: Vec<Item>,
    /// What is on screen, in order.
    rows: Vec<Row>,
    /// The group being looked inside. Empty is the top level.
    path: Vec<String>,
    query: String,
    /// Index into `rows`.
    cursor: usize,
    /// First visible row.
    top: usize,
    hover: Option<usize>,
    /// Whether any row has a picture, which is what reserves the column.
    has_icons: bool,
    anchor: Option<(i32, i32)>,
    width: i32,
    result: Option<usize>,
    accepted: bool,
    palette: &'static Palette,
    m: Metrics,
    icons: RefCell<IconCache>,
    font: Option<crate::dialog::UiFont>,
    /// Segoe MDL2, for the glyph column and the chevrons.
    glyphs: HFONT,
}

impl State {
    fn rows_visible(&self, height: i32) -> usize {
        ((height - self.m.query_h) / self.m.row_h).max(1) as usize
    }

    /// Rebuild what is on screen from the query and the group we are in.
    ///
    /// With a query the whole list is searched flat, groups and all: typing
    /// "sha" finds `7-Zip › CRC SHA › SHA-256` without anyone having to know
    /// it lives three levels down. With no query it is one level at a time,
    /// the way a menu is.
    fn rebuild(&mut self) {
        self.rows.clear();
        if !self.query.is_empty() {
            self.rows
                .extend(filter(&self.items, &self.query).into_iter().map(Row::Item));
        } else {
            let labels: Vec<String> = self.items.iter().map(|i| i.label.clone()).collect();
            let (leaves, groups) = level(&labels, &self.path);
            if !self.path.is_empty() {
                self.rows.push(Row::Back);
            }
            self.rows.extend(leaves.into_iter().map(Row::Item));
            self.rows
                .extend(groups.into_iter().map(|(n, c)| Row::Group(n, c)));
        }
        self.cursor = 0;
        self.top = 0;
    }

    /// The label a row shows, which inside a group is only the part below it.
    fn row_label(&self, row: &Row) -> String {
        match row {
            Row::Item(i) => {
                let label = &self.items[*i].label;
                if self.query.is_empty() && !self.path.is_empty() {
                    let prefix = format!("{}{}", self.path.join(GROUP_SEP), GROUP_SEP);
                    label.strip_prefix(&prefix).unwrap_or(label).to_string()
                } else {
                    label.clone()
                }
            }
            Row::Group(name, _) => name.clone(),
            Row::Back => self.path.last().cloned().unwrap_or_default(),
        }
    }

    /// Keep the cursor on screen, which is the only thing scrolling is for.
    fn scroll_into_view(&mut self, height: i32) {
        let rows = self.rows_visible(height);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + rows {
            self.top = self.cursor + 1 - rows;
        }
        let max_top = self.rows.len().saturating_sub(rows);
        self.top = self.top.min(max_top);
    }
}

/// Show the picker in the middle of the window. Returns the chosen index.
pub fn pick(
    parent: HWND,
    dpi: u32,
    theme: Theme,
    title: &str,
    items: Vec<Item>,
) -> Option<usize> {
    unsafe { pick_impl(parent, dpi, theme, title, items, None) }
}

/// Show it at a point on screen, the way a menu appears where you clicked.
///
/// Anchored means no caption and no title — a context menu with a title bar is
/// a dialog — and a height that follows the number of rows rather than leaving
/// an empty box under a short list.
pub fn pick_at(
    parent: HWND,
    dpi: u32,
    theme: Theme,
    items: Vec<Item>,
    anchor: (i32, i32),
) -> Option<usize> {
    unsafe { pick_impl(parent, dpi, theme, "", items, Some(anchor)) }
}

unsafe fn pick_impl(
    parent: HWND,
    dpi: u32,
    theme: Theme,
    title: &str,
    items: Vec<Item>,
    anchor: Option<(i32, i32)>,
) -> Option<usize> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = wide(CLASS);
    let p = theme.palette();

    // Registering twice is harmless: the second call fails and we carry on.
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(proc_),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);

    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale).round() as i32;
    let m = Metrics::new(dpi);
    let w = s(520.0);

    let st = Box::into_raw(Box::new(State {
        has_icons: items.iter().any(|i| {
            i.icon.is_some() || i.bitmap.is_some()
        }),
        items,
        rows: Vec::new(),
        path: Vec::new(),
        query: String::new(),
        cursor: 0,
        top: 0,
        hover: None,
        anchor,
        width: w,
        result: None,
        accepted: false,
        palette: p,
        m,
        icons: RefCell::new(IconCache::default()),
        font: crate::dialog::UiFont::new(dpi),
        glyphs: glyph_font(dpi),
    }));
    (*st).rebuild();

    let (x, y, h) = match anchor {
        None => {
            let h = s(420.0);
            let mut pr = RECT::default();
            let _ = GetWindowRect(parent, &mut pr);
            (
                pr.left + ((pr.right - pr.left) - w) / 2,
                pr.top + ((pr.bottom - pr.top) - h) / 4,
                h,
            )
        }
        Some(at) => anchored_box(at, (*st).rows.len(), &(*st).m, w),
    };

    let title_w = wide(title);
    let style = if anchor.is_some() {
        WS_POPUP | WS_BORDER
    } else {
        WS_POPUPWINDOW | WS_CAPTION
    };
    let dlg = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title_w.as_ptr()),
        style,
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

    crate::app::apply_titlebar_theme(dlg, theme);
    round_corners(dlg);
    let _ = EnableWindow(parent, false);
    let _ = ShowWindow(dlg, SW_SHOW);
    // An anchored picker is a context menu, and a menu goes away when you
    // click past it. The parent is disabled, so that click is never delivered
    // there: capture the mouse the way a menu does and the click arrives here
    // instead, where the handler can tell inside from outside. Not for the
    // centred palette, which has a caption its owner drags by.
    if anchor.is_some() {
        SetCapture(dlg);
    }

    let mut quit_code: Option<i32> = None;
    let mut msg = MSG::default();

    while IsWindow(Some(dlg)).as_bool() {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if r.0 == -1 {
            break;
        }
        if r.0 == 0 {
            // WM_QUIT arrived while modal: remember it and re-post on the way
            // out, or the app can never be closed from here.
            quit_code = Some(msg.wParam.0 as i32);
            break;
        }

        // The query box has focus, so the keys that drive the list have to be
        // taken before it sees them.
        if msg.message == WM_KEYDOWN {
            let vk = msg.wParam.0 as u16;
            let handled = match vk {
                0x26 => step(dlg, -1),           // Up
                0x28 => step(dlg, 1),            // Down
                0x21 => step(dlg, -10),          // PageUp
                0x22 => step(dlg, 10),           // PageDown
                0x24 => step(dlg, i32::MIN / 2), // Home
                0x23 => step(dlg, i32::MAX / 2), // End
                0x27 => enter_group(dlg),        // Right
                0x25 => leave_group(dlg),        // Left
                0x0D => {
                    accept(dlg);
                    true
                }
                0x1B => {
                    // Escape leaves the group first, and closes only when
                    // there is nowhere left to go back to.
                    if !leave_group(dlg) {
                        let _ = DestroyWindow(dlg);
                    }
                    true
                }
                _ => false,
            };
            if handled {
                continue;
            }
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
    let _ = DeleteObject(HGDIOBJ(st.glyphs.0));
    if let Some(code) = quit_code {
        PostQuitMessage(code);
    }
    if st.accepted {
        st.result
    } else {
        None
    }
}

/// The icon font, at the size the rows are drawn in.
unsafe fn glyph_font(dpi: u32) -> HFONT {
    let face = wide("Segoe MDL2 Assets");
    CreateFontW(
        -((12.0 * dpi as f32 / 96.0).round() as i32),
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(face.as_ptr()),
    )
}

/// Windows 11 rounds popup corners for its own menus; ours asks for the same.
/// Older builds ignore the attribute, which is the whole error handling here.
unsafe fn round_corners(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE};
    let round: u32 = 2; // DWMWCP_ROUND
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &round as *const _ as *const _,
        std::mem::size_of::<u32>() as u32,
    );
}

unsafe fn state_of(dlg: HWND) -> Option<&'static mut State> {
    let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

unsafe fn client_height(dlg: HWND) -> i32 {
    let mut r = RECT::default();
    let _ = GetClientRect(dlg, &mut r);
    r.bottom - r.top
}

/// Move the cursor `by` rows. Returns true so the caller can stop the key.
unsafe fn step(dlg: HWND, by: i32) -> bool {
    let h = client_height(dlg);
    let Some(st) = state_of(dlg) else { return false };
    if st.rows.is_empty() {
        return true;
    }
    let last = st.rows.len() as i32 - 1;
    st.cursor = (st.cursor as i32 + by).clamp(0, last) as usize;
    st.scroll_into_view(h);
    let _ = InvalidateRect(Some(dlg), None, false);
    true
}

/// Go into the group under the cursor. False when it is not a group.
unsafe fn enter_group(dlg: HWND) -> bool {
    let Some(st) = state_of(dlg) else { return false };
    let Some(Row::Group(name, _)) = st.rows.get(st.cursor) else {
        return false;
    };
    let name = name.clone();
    st.path.push(name);
    st.rebuild();
    resize_to_rows(dlg);
    let _ = InvalidateRect(Some(dlg), None, false);
    true
}

/// Come back out of a group. False at the top level, where the key means
/// whatever it meant before.
unsafe fn leave_group(dlg: HWND) -> bool {
    let Some(st) = state_of(dlg) else { return false };
    if st.path.is_empty() {
        return false;
    }
    st.path.pop();
    st.rebuild();
    resize_to_rows(dlg);
    let _ = InvalidateRect(Some(dlg), None, false);
    true
}

/// An anchored picker is as tall as its list, so it follows the filter.
unsafe fn resize_to_rows(dlg: HWND) {
    let Some(st) = state_of(dlg) else { return };
    let Some(at) = st.anchor else { return };
    let (x, y, want) = anchored_box(at, st.rows.len(), &st.m, st.width);
    let _ = SetWindowPos(
        dlg,
        None,
        x,
        y,
        st.width,
        want,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

unsafe fn accept(dlg: HWND) {
    let Some(st) = state_of(dlg) else { return };
    match st.rows.get(st.cursor) {
        Some(Row::Item(index)) => {
            st.result = Some(*index);
            st.accepted = true;
        }
        // Enter on a group or on the way back is navigation, not a choice.
        Some(Row::Group(..)) => {
            enter_group(dlg);
            return;
        }
        Some(Row::Back) => {
            leave_group(dlg);
            return;
        }
        None => return,
    }
    let _ = DestroyWindow(dlg);
}

unsafe fn refill(dlg: HWND, query: &str) {
    let h = client_height(dlg);
    let Some(st) = state_of(dlg) else { return };
    st.query = query.to_string();
    st.rebuild();
    resize_to_rows(dlg);
    let h = if st.anchor.is_some() {
        client_height(dlg)
    } else {
        h
    };
    st.scroll_into_view(h);
    let _ = InvalidateRect(Some(dlg), None, false);
}

/// Which row a point is over.
unsafe fn row_at(st: &State, y: i32) -> Option<usize> {
    if y < st.m.query_h {
        return None;
    }
    let i = st.top + ((y - st.m.query_h) / st.m.row_h) as usize;
    (i < st.rows.len()).then_some(i)
}

unsafe fn fill(dc: HDC, r: RECT, c: Rgb) {
    let brush = CreateSolidBrush(colorref(c));
    FillRect(dc, &r, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

/// A rounded rectangle, for the selected row and the accelerator chips.
unsafe fn fill_rounded(dc: HDC, r: RECT, radius: i32, c: Rgb) {
    let brush = CreateSolidBrush(colorref(c));
    let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, radius * 2, radius * 2);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

unsafe fn draw_text(dc: HDC, text: &str, mut r: RECT, flags: DRAW_TEXT_FORMAT) {
    if text.is_empty() {
        return;
    }
    let w = wide(text);
    DrawTextW(dc, &mut w.clone()[..w.len() - 1], &mut r, flags);
}

unsafe fn paint(dlg: HWND) {
    let Some(st) = state_of(dlg) else { return };
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(dlg, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(dlg, &mut rc);
    let p = st.palette;
    let m = &st.m;

    // Everything is drawn into a bitmap and blitted once: a list that repaints
    // on every keystroke flickers otherwise.
    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
    let old_bmp = SelectObject(mem, HGDIOBJ(bmp.0));

    fill(mem, rc, p.pane_bg);
    fill(
        mem,
        RECT {
            left: 0,
            top: m.query_h - 1,
            right: rc.right,
            bottom: m.query_h,
        },
        p.divider,
    );

    let text_font = st
        .font
        .as_ref()
        .map(|f| f.handle())
        .unwrap_or(HFONT(std::ptr::null_mut()));
    let old_font = SelectObject(mem, HGDIOBJ(text_font.0));
    SetBkMode(mem, TRANSPARENT);

    // The magnifier, which is what says the box is a search box.
    SelectObject(mem, HGDIOBJ(st.glyphs.0));
    SetTextColor(mem, colorref(p.text_faint));
    draw_text(
        mem,
        GLYPH_SEARCH,
        RECT {
            left: m.pad,
            top: 0,
            right: m.pad + m.icon,
            bottom: m.query_h,
        },
        DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX,
    );
    SelectObject(mem, HGDIOBJ(text_font.0));

    let text_x = m.pad + if st.has_icons { m.icon + m.pad } else { 0 };
    let rows = st.rows_visible(rc.bottom);
    for slot in 0..rows {
        let Some(row) = st.rows.get(st.top + slot) else {
            break;
        };
        let y = m.query_h + slot as i32 * m.row_h;
        let full = RECT {
            left: 0,
            top: y,
            right: rc.right,
            bottom: y + m.row_h,
        };
        let is_cursor = st.top + slot == st.cursor;
        let pill = RECT {
            left: m.pad / 2,
            top: y + 1,
            right: rc.right - m.pad / 2,
            bottom: full.bottom - 1,
        };
        if is_cursor {
            fill_rounded(mem, pill, m.radius, p.selection);
        } else if st.hover == Some(st.top + slot) {
            fill_rounded(mem, pill, m.radius, p.row_hover);
        }

        let fg = if is_cursor { p.text_on_selection } else { p.text };
        let icon_y = y + (m.row_h - m.icon) / 2;

        // The picture: a handler's bitmap, a file icon, one of our glyphs, or
        // the arrow on the way out of a group.
        match row {
            Row::Back => {
                SelectObject(mem, HGDIOBJ(st.glyphs.0));
                SetTextColor(mem, colorref(fg));
                draw_text(
                    mem,
                    GLYPH_BACK,
                    RECT {
                        left: m.pad,
                        top: y,
                        right: m.pad + m.icon,
                        bottom: full.bottom,
                    },
                    DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX,
                );
                SelectObject(mem, HGDIOBJ(text_font.0));
            }
            Row::Item(i) => {
                let item = &st.items[*i];
                if let Some(b) = item.bitmap {
                    draw_bitmap(mem, b, m.pad, icon_y, m.icon);
                } else if let Some(key) = &item.icon {
                    if let Some(icon) = st.icons.borrow_mut().get(key) {
                        let _ = DrawIconEx(
                            mem, m.pad, icon_y, icon, m.icon, m.icon, 0, None, DI_NORMAL,
                        );
                    }
                }
            }
            Row::Group(..) => {}
        }

        let label = st.row_label(row);
        SetTextColor(mem, colorref(fg));
        draw_text(
            mem,
            &label,
            RECT {
                left: text_x,
                top: y,
                right: rc.right - m.pad * 2,
                bottom: full.bottom,
            },
            DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX | DT_END_ELLIPSIS,
        );

        // The matched run, underlined in the accent colour. Redrawing the text
        // in another colour would need the exact width of the part before it,
        // and a line under it says the same thing with one rectangle.
        if let Some((a, b)) = highlight(&label, &st.query) {
            let before: String = label.chars().take(a).collect();
            let run: String = label.chars().skip(a).take(b - a).collect();
            let w0 = text_width(mem, &before);
            let w1 = text_width(mem, &run);
            fill(
                mem,
                RECT {
                    left: text_x + w0,
                    top: full.bottom - 3,
                    right: text_x + w0 + w1,
                    bottom: full.bottom - 1,
                },
                if is_cursor { p.text_on_selection } else { p.accent },
            );
        }

        match row {
            // A chevron says the row opens rather than acts.
            Row::Group(_, count) => {
                SelectObject(mem, HGDIOBJ(st.glyphs.0));
                SetTextColor(mem, colorref(if is_cursor { fg } else { p.text_faint }));
                draw_text(
                    mem,
                    GLYPH_CHEVRON,
                    RECT {
                        left: 0,
                        top: y,
                        right: rc.right - m.pad,
                        bottom: full.bottom,
                    },
                    DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
                );
                SelectObject(mem, HGDIOBJ(text_font.0));
                let _ = count;
            }
            // The shortcut, in a chip: a bare grey string at the right-hand
            // end reads as part of the label.
            Row::Item(i) if !st.items[*i].detail.is_empty() => {
                let detail = &st.items[*i].detail;
                let w = text_width(mem, detail);
                let chip = RECT {
                    left: rc.right - m.pad - w - m.pad,
                    top: y + 3,
                    right: rc.right - m.pad,
                    bottom: full.bottom - 3,
                };
                if !is_cursor {
                    fill_rounded(mem, chip, m.radius / 2, p.field_bg);
                }
                SetTextColor(
                    mem,
                    colorref(if is_cursor { fg } else { p.text_faint }),
                );
                draw_text(
                    mem,
                    detail,
                    RECT {
                        left: chip.left,
                        top: y,
                        right: chip.right - m.pad / 2,
                        bottom: full.bottom,
                    },
                    DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
                );
            }
            _ => {}
        }
    }

    if st.rows.is_empty() {
        SetTextColor(mem, colorref(p.text_faint));
        draw_text(
            mem,
            "Nothing matches",
            RECT {
                left: 0,
                top: m.query_h,
                right: rc.right,
                bottom: rc.bottom,
            },
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
        );
    }

    let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(mem);
    let _ = EndPaint(dlg, &ps);
}

/// Where an anchored picker sits for a list of `rows`.
///
/// Below the point if it fits, above if it fits there instead, and otherwise
/// as much as the roomier side has. It stays attached to the click either way:
/// a long list used to flip to the top of the screen, which is nowhere near
/// where anybody pressed the button.
unsafe fn anchored_box(at: (i32, i32), rows: usize, m: &Metrics, w: i32) -> (i32, i32, i32) {
    let (ax, ay) = at;
    // A whole pad of slack: the border eats a pixel at each end, and a row
    // clipped by two is a row that does not get drawn at all.
    let wanted = m.query_h + rows as i32 * m.row_h + m.pad;
    let work = monitor_work_area(ax, ay);
    let floor = m.query_h + m.row_h;

    let below = (work.bottom - ay).max(0);
    let above = (ay - work.top).max(0);
    let (y, h) = if wanted <= below {
        (ay, wanted)
    } else if wanted <= above {
        (ay - wanted, wanted)
    } else if below >= above {
        (ay, below.max(floor))
    } else {
        (work.top, above.max(floor))
    };
    let x = ax.min(work.right - w).max(work.left);
    (x, y.max(work.top), h)
}

/// The usable part of the monitor a point is on, so a menu opened near an
/// edge lands on screen rather than half off it.
unsafe fn monitor_work_area(x: i32, y: i32) -> RECT {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let mon = MonitorFromPoint(
        windows::Win32::Foundation::POINT { x, y },
        MONITOR_DEFAULTTONEAREST,
    );
    if GetMonitorInfoW(mon, &mut info).as_bool() {
        info.rcWork
    } else {
        RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        }
    }
}

/// Draw somebody else's menu bitmap, scaled into a square.
///
/// Through AlphaBlend because a shell handler's bitmap is usually 32-bit with
/// a premultiplied alpha channel; blitting it straight leaves a black box
/// around the picture.
unsafe fn draw_bitmap(dc: HDC, bmp: HBITMAP, x: i32, y: i32, size: i32) {
    let mut bm = BITMAP::default();
    let got = GetObjectW(
        HGDIOBJ(bmp.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    );
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        return;
    }
    let src = CreateCompatibleDC(Some(dc));
    let old = SelectObject(src, HGDIOBJ(bmp.0));
    if bm.bmBitsPixel == 32 {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(dc, x, y, size, size, src, 0, 0, bm.bmWidth, bm.bmHeight, blend);
    } else {
        let _ = StretchBlt(dc, x, y, size, size, Some(src), 0, 0, bm.bmWidth, bm.bmHeight, SRCCOPY);
    }
    SelectObject(src, old);
    let _ = DeleteDC(src);
}

unsafe fn text_width(dc: HDC, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &w, &mut size);
    size.cx
}

/// Is a mouse message's point inside the window? With the mouse captured it
/// may be anywhere on screen, and a negative coordinate arrives as a large
/// unsigned one, so the halves are read as signed.
unsafe fn over_window(hwnd: HWND, lparam: LPARAM) -> bool {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    x >= 0 && y >= 0 && x < rc.right && y < rc.bottom
}

extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let st = cs.lpCreateParams as *mut State;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, st as isize);
                let m = &(*st).m;

                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let edit = wide("Edit");
                let left = m.pad + m.icon + m.pad / 2;
                let e = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(edit.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    left,
                    m.pad / 2,
                    rc.right - left - m.pad,
                    m.query_h - m.pad,
                    Some(hwnd),
                    Some(HMENU(IDC_QUERY as *mut _)),
                    Some(cs.hInstance),
                    None,
                );
                if let Ok(e) = &e {
                    if let Some(f) = (*st).font.as_ref() {
                        SendMessageW(
                            *e,
                            WM_SETFONT,
                            Some(WPARAM(f.handle().0 as usize)),
                            Some(LPARAM(1)),
                        );
                    }
                    let cue = wide("Select a command\u{2026}");
                    SendMessageW(
                        *e,
                        EM_SETCUEBANNER,
                        Some(WPARAM(1)),
                        Some(LPARAM(cue.as_ptr() as isize)),
                    );
                    let _ = SetFocus(Some(*e));
                }
                LRESULT(0)
            }

            // The query box is a real EDIT, so it comes with the system's
            // colours until it is told otherwise.
            WM_CTLCOLOREDIT => {
                let Some(st) = state_of(hwnd) else {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                };
                let dc = HDC(wparam.0 as *mut _);
                SetTextColor(dc, colorref(st.palette.text));
                SetBkColor(dc, colorref(st.palette.pane_bg));
                // Leaked deliberately once per picker: the brush has to outlive
                // the message, and the window is gone moments later.
                LRESULT(CreateSolidBrush(colorref(st.palette.pane_bg)).0 as isize)
            }

            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }

            WM_MOUSEMOVE => {
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let over = over_window(hwnd, lparam);
                if let Some(st) = state_of(hwnd) {
                    let was = st.hover;
                    st.hover = over.then(|| row_at(st, y)).flatten();
                    if was != st.hover {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }

            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                if !over_window(hwnd, lparam) {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                if msg == WM_RBUTTONDOWN {
                    return LRESULT(0);
                }
                if let Some(st) = state_of(hwnd) {
                    if let Some(row) = row_at(st, y) {
                        st.cursor = row;
                        accept(hwnd);
                        return LRESULT(0);
                    }
                }
                LRESULT(0)
            }

            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let h = client_height(hwnd);
                if let Some(st) = state_of(hwnd) {
                    let rows = st.rows_visible(h);
                    let max_top = st.rows.len().saturating_sub(rows);
                    let by = -(delta / 120) * 3;
                    st.top = (st.top as i32 + by).clamp(0, max_top as i32) as usize;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }

            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDC_QUERY && code == EN_CHANGE {
                    if let Ok(q) = GetDlgItem(Some(hwnd), IDC_QUERY) {
                        let len = GetWindowTextLengthW(q).max(0) as usize;
                        let mut buf = vec![0u16; len + 1];
                        let n = GetWindowTextW(q, &mut buf);
                        let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
                        refill(hwnd, &text);
                    }
                }
                LRESULT(0)
            }

            // Losing the mouse to another window is how a menu learns the
            // user has moved on. Sitting there with a disabled parent behind
            // is the state this capture exists to avoid.
            WM_CAPTURECHANGED => {
                if state_of(hwnd).is_some_and(|st| st.anchor.is_some()) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            // The caller owns the box; the window only clears its pointer.
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<Item> {
        labels
            .iter()
            .map(|l| Item {
                label: (*l).to_string(),
                detail: String::new(),
                icon: None,
                bitmap: None,
            })
            .collect()
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_query_keeps_every_item_in_order() {
        let it = items(&["Zebra", "Apple", "Mango"]);
        assert_eq!(filter(&it, ""), vec![0, 1, 2]);
    }

    #[test]
    fn initials_find_a_multi_word_command() {
        let it = items(&["New folder", "Refresh", "Rename"]);
        let got = filter(&it, "nf");
        assert_eq!(it[got[0]].label, "New folder");
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert_eq!(score("New folder", "xyz"), None);
        assert_eq!(score("New folder", "fn"), None, "order matters");
    }

    #[test]
    fn word_starts_beat_mid_word_hits() {
        let start = score("Copy path", "cp").unwrap();
        let middle = score("Recompute", "cp").unwrap();
        assert!(start > middle, "{} should beat {}", start, middle);
    }

    #[test]
    fn a_prefix_match_beats_a_scattered_one() {
        // "Rename file" matches r-e-f across two word starts, which without a
        // prefix bonus outscored the obvious answer.
        let run = score("Refresh", "ref").unwrap();
        let scattered = score("Rename file", "ref").unwrap();
        assert!(run > scattered, "{} should beat {}", run, scattered);
    }

    #[test]
    fn exact_prefix_ranks_first() {
        let it = items(&["Paste", "Properties", "Open in new tab"]);
        let got = filter(&it, "pa");
        assert_eq!(it[got[0]].label, "Paste");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("New Folder", "NEWF").is_some());
        assert!(score("new folder", "NF").is_some());
    }

    #[test]
    fn the_highlighted_run_is_where_the_query_actually_appears() {
        assert_eq!(highlight("7-Zip \u{203a} Add to archive", "zip"), Some((2, 5)));
        assert_eq!(highlight("Refresh", "REF"), Some((0, 3)));
        // A subsequence match that is not a run has nothing to underline, and
        // the row is still in the list because `score` is the one that decides.
        assert_eq!(highlight("New folder", "nf"), None);
        assert_eq!(highlight("short", "longer query"), None);
        assert_eq!(highlight("anything", ""), None);
    }

    #[test]
    fn one_level_shows_leaves_here_and_groups_for_everything_deeper() {
        let labels = strs(&[
            "Open",
            "Cut",
            "7-Zip \u{203a} Extract here",
            "7-Zip \u{203a} Add to archive",
            "7-Zip \u{203a} CRC SHA \u{203a} SHA-256",
            "Send to \u{203a} Desktop",
        ]);
        let (leaves, groups) = level(&labels, &[]);
        assert_eq!(leaves, vec![0, 1], "Open and Cut are here");
        assert_eq!(
            groups,
            vec![("7-Zip".to_string(), 3), ("Send to".to_string(), 1)],
            "and everything deeper is behind its own name"
        );
    }

    #[test]
    fn going_into_a_group_shows_what_is_directly_inside_it() {
        let labels = strs(&[
            "Open",
            "7-Zip \u{203a} Extract here",
            "7-Zip \u{203a} CRC SHA \u{203a} SHA-256",
            "7-Zip \u{203a} CRC SHA \u{203a} CRC-32",
        ]);
        let (leaves, groups) = level(&labels, &strs(&["7-Zip"]));
        assert_eq!(leaves, vec![1], "Extract here, by index into the whole list");
        assert_eq!(groups, vec![("CRC SHA".to_string(), 2)]);

        // And one deeper still, where there is nothing left to group.
        let (leaves, groups) = level(&labels, &strs(&["7-Zip", "CRC SHA"]));
        assert_eq!(leaves, vec![2, 3]);
        assert!(groups.is_empty());
    }

    #[test]
    fn a_group_that_does_not_exist_is_simply_empty() {
        let labels = strs(&["Open", "7-Zip \u{203a} Extract here"]);
        let (leaves, groups) = level(&labels, &strs(&["Nothing"]));
        assert!(leaves.is_empty() && groups.is_empty());
    }
}
