// Direct2D / DirectWrite painting.
//
// Three things here are deliberate departures from the previous version:
//
//  * One reusable solid brush whose colour is set per draw, instead of eleven
//    cached brushes torn down and rebuilt on every WM_SIZE.
//  * `Resize` on the existing render target rather than destroying and
//    recreating it, so dragging the window edge does not thrash the GPU.
//  * Text formats carry no-wrap, ellipsis trimming, and vertical centring, so a
//    long filename is trimmed with an ellipsis instead of wrapping inside a
//    24px row and being clipped mid-glyph.
//
// The render target runs at 96 DPI so one DIP is one physical pixel; scaling
// for the monitor happens once, in `layout::Metrics`, and in the font sizes
// below. That keeps every coordinate in the app in the same unit.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{Interface, Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::file_list::{FileList, SortKey, SortOrder};
use crate::icons::{icon_key, IconCache};
use crate::layout::{Hit, Layout, PaneLayout, Rect, Side, TextMeasurer};
use crate::theme::{Palette, Rgb, Theme};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn d2d_rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x as f32,
        top: r.y as f32,
        right: r.right() as f32,
        bottom: r.bottom() as f32,
    }
}

fn color(c: Rgb) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.0,
        g: c.1,
        b: c.2,
        a: 1.0,
    }
}

/// Everything the renderer needs to paint one pane.
pub struct PaneView<'a> {
    pub side: Side,
    pub layout: &'a PaneLayout,
    pub tab_labels: &'a [String],
    pub active_tab: usize,
    pub crumbs: &'a [String],
    pub list: &'a FileList,
    pub focused: bool,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub hover: Option<Hit>,
}

struct Formats {
    body: IDWriteTextFormat,
    body_right: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_bold: IDWriteTextFormat,
}

pub struct Renderer {
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    target: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    formats: Formats,
    theme: Theme,
    dpi: u32,

    icons: IconCache,
    /// Shell icons converted to D2D bitmaps. Cleared whenever the render target
    /// is recreated, because the bitmaps belong to that target.
    icon_bitmaps: HashMap<String, Option<ID2D1Bitmap>>,
    /// Memo for text measurement: layout asks for the same tab and crumb
    /// widths every frame.
    measure_cache: RefCell<HashMap<(String, bool), f32>>,
}

impl Renderer {
    pub fn new(theme: Theme, dpi: u32) -> Result<Self> {
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let formats = Self::make_formats(&dwrite, dpi)?;
        Ok(Self {
            factory,
            dwrite,
            wic,
            target: None,
            brush: None,
            formats,
            theme,
            dpi,
            icons: IconCache::new(),
            icon_bitmaps: HashMap::new(),
            measure_cache: RefCell::new(HashMap::new()),
        })
    }

    fn make_formats(dwrite: &IDWriteFactory, dpi: u32) -> Result<Formats> {
        let scale = dpi as f32 / 96.0;
        let font = wide("Segoe UI");
        let locale = wide("en-us");

        let mk = |size: f32, weight: DWRITE_FONT_WEIGHT, align: DWRITE_TEXT_ALIGNMENT| -> Result<IDWriteTextFormat> {
            let f = unsafe {
                dwrite.CreateTextFormat(
                    PCWSTR::from_raw(font.as_ptr()),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size * scale,
                    PCWSTR::from_raw(locale.as_ptr()),
                )?
            };
            unsafe {
                // A row is one line: never wrap, trim with an ellipsis instead.
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                // Centre the text in its row rather than pinning it to the top,
                // which is what made every row look misaligned before.
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetTextAlignment(align)?;
                let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                f.SetTrimming(&trimming, &sign)?;
            }
            Ok(f)
        };

        Ok(Formats {
            body: mk(13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            body_right: mk(13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            small: mk(12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            small_bold: mk(12.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        })
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    fn palette(&self) -> &'static Palette {
        self.theme.palette()
    }

    /// Recreate the text formats at a new DPI. Metrics are rescaled by the caller.
    pub fn set_dpi(&mut self, dpi: u32) -> Result<()> {
        if dpi == self.dpi {
            return Ok(());
        }
        self.dpi = dpi;
        self.formats = Self::make_formats(&self.dwrite, dpi)?;
        self.measure_cache.borrow_mut().clear();
        Ok(())
    }

    /// Create the render target, or resize the existing one.
    ///
    /// `Resize` keeps the target, its brush, and every cached icon bitmap alive.
    /// The old code dropped and rebuilt all of it on every WM_SIZE, which is why
    /// dragging the window edge stuttered.
    pub fn resize(&mut self, hwnd: HWND, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if let Some(target) = &self.target {
            let size = D2D_SIZE_U { width, height };
            unsafe { target.Resize(&size)? };
            return Ok(());
        }

        let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
            // 1 DIP == 1 physical pixel; DPI scaling is applied in Metrics.
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U { width, height },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let target = unsafe { self.factory.CreateHwndRenderTarget(&rt_props, &hwnd_props)? };
        let brush = unsafe {
            target.CreateSolidColorBrush(&color((1.0, 1.0, 1.0)), None)?
        };
        self.brush = Some(brush);
        self.target = Some(target);
        self.icon_bitmaps.clear();
        Ok(())
    }

    /// Drop the device-dependent resources after D2DERR_RECREATE_TARGET.
    pub fn discard_target(&mut self) {
        self.target = None;
        self.brush = None;
        self.icon_bitmaps.clear();
    }

    pub fn has_target(&self) -> bool {
        self.target.is_some()
    }

    // -- primitives --------------------------------------------------------

    fn rt(&self) -> Option<&ID2D1HwndRenderTarget> {
        self.target.as_ref()
    }

    fn fill(&self, r: Rect, c: Rgb) {
        if r.is_empty() {
            return;
        }
        let (Some(rt), Some(brush)) = (self.rt(), self.brush.as_ref()) else {
            return;
        };
        unsafe {
            brush.SetColor(&color(c));
            rt.FillRectangle(&d2d_rect(r), brush);
        }
    }

    fn stroke(&self, r: Rect, c: Rgb, width: f32) {
        if r.is_empty() {
            return;
        }
        let (Some(rt), Some(brush)) = (self.rt(), self.brush.as_ref()) else {
            return;
        };
        // Inset by half the stroke so the outline lands inside the rect.
        let half = width / 2.0;
        let rr = D2D_RECT_F {
            left: r.x as f32 + half,
            top: r.y as f32 + half,
            right: r.right() as f32 - half,
            bottom: r.bottom() as f32 - half,
        };
        unsafe {
            brush.SetColor(&color(c));
            rt.DrawRectangle(&rr, brush, width, None);
        }
    }

    fn text(&self, s: &str, r: Rect, c: Rgb, fmt: &IDWriteTextFormat) {
        if s.is_empty() || r.is_empty() {
            return;
        }
        let (Some(rt), Some(brush)) = (self.rt(), self.brush.as_ref()) else {
            return;
        };
        let buf: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            brush.SetColor(&color(c));
            rt.DrawText(
                &buf,
                fmt,
                &d2d_rect(r),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn push_clip(&self, r: Rect) {
        if let Some(rt) = self.rt() {
            unsafe { rt.PushAxisAlignedClip(&d2d_rect(r), D2D1_ANTIALIAS_MODE_ALIASED) };
        }
    }

    fn pop_clip(&self) {
        if let Some(rt) = self.rt() {
            unsafe { rt.PopAxisAlignedClip() };
        }
    }

    // -- icons -------------------------------------------------------------

    fn icon_bitmap(&mut self, key: &str) -> Option<&ID2D1Bitmap> {
        if !self.icon_bitmaps.contains_key(key) {
            let hicon = self.icons.get(key);
            let bmp = hicon.and_then(|h| self.hicon_to_bitmap(h).ok());
            self.icon_bitmaps.insert(key.to_string(), bmp);
        }
        self.icon_bitmaps.get(key).and_then(|o| o.as_ref())
    }

    fn hicon_to_bitmap(&self, hicon: HICON) -> Result<ID2D1Bitmap> {
        let rt = self
            .rt()
            .ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        unsafe {
            let src = self.wic.CreateBitmapFromHICON(hicon)?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(
                &src,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            let bitmap = rt.CreateBitmapFromWicBitmap(&converter, None)?;
            Ok(bitmap.cast()?)
        }
    }

    fn draw_icon(&mut self, key: &str, r: Rect) {
        let dest = d2d_rect(r);
        // Borrow the target before the mutable icon lookup so both can coexist.
        let Some(bmp) = self.icon_bitmap(key).cloned() else {
            return;
        };
        if let Some(rt) = self.rt() {
            unsafe {
                rt.DrawBitmap(
                    &bmp,
                    Some(&dest),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
    }

    // -- frame -------------------------------------------------------------

    pub fn begin(&self) {
        if let Some(rt) = self.rt() {
            unsafe { rt.BeginDraw() };
            let bg = color(self.palette().window_bg);
            unsafe { rt.Clear(Some(&bg)) };
        }
    }

    /// Finish the frame. Returns Err when the device was lost and the caller
    /// should discard the target and repaint.
    pub fn end(&self) -> Result<()> {
        if let Some(rt) = self.rt() {
            unsafe { rt.EndDraw(None, None)? };
        }
        Ok(())
    }

    // -- chrome ------------------------------------------------------------

    pub fn draw_sidebar(&mut self, layout: &Layout, drives: &[crate::fs::Drive], current: &str, hover: Option<Hit>) {
        if layout.sidebar.is_empty() {
            return;
        }
        let p = self.palette();
        let m = layout.metrics;
        self.fill(layout.sidebar, p.sidebar_bg);

        let title = Rect::new(
            layout.sidebar.x + m.pad,
            layout.sidebar.y,
            layout.sidebar.w - m.pad,
            m.sidebar_header_h,
        );
        self.text("DRIVES", title, p.text_muted, &self.formats.small_bold.clone());

        for (i, r) in layout.drives.clone().iter().enumerate() {
            let Some(drive) = drives.get(i) else { continue };
            let is_current = current.to_lowercase().starts_with(&drive.root.to_lowercase());
            if is_current {
                self.fill(*r, p.selection_inactive);
            } else if hover == Some(Hit::Drive(i)) {
                self.fill(*r, p.row_hover);
            }
            let icon = Rect::new(
                r.x + m.pad,
                r.y + (r.h - m.icon_size) / 2,
                m.icon_size,
                m.icon_size,
            );
            self.draw_icon("<dir>", icon);
            let label = Rect::new(
                icon.right() + m.pad,
                r.y,
                r.right() - icon.right() - m.pad * 2,
                r.h,
            );
            self.text(&drive.display(), label, p.text, &self.formats.small.clone());
        }
    }

    pub fn draw_divider(&mut self, layout: &Layout, hovered: bool, dragging: bool) {
        let p = self.palette();
        let c = if dragging {
            p.focus
        } else if hovered {
            p.scrollbar_thumb_hover
        } else {
            p.divider
        };
        self.fill(layout.divider, p.window_bg);
        // A 1px line down the middle of the (wider) grab area.
        let mid = Rect::new(
            layout.divider.x + layout.divider.w / 2,
            layout.divider.y,
            1.max(layout.divider.w / 6),
            layout.divider.h,
        );
        self.fill(mid, c);
    }

    pub fn draw_status(&mut self, layout: &Layout, text: &str, right_text: &str) {
        let p = self.palette();
        let m = layout.metrics;
        self.fill(layout.status, p.status_bg);
        let top = Rect::new(layout.status.x, layout.status.y, layout.status.w, 1);
        self.fill(top, p.divider);

        let left = Rect::new(
            layout.status.x + m.pad,
            layout.status.y,
            layout.status.w / 2,
            layout.status.h,
        );
        self.text(text, left, p.text_muted, &self.formats.small.clone());

        let right = Rect::new(
            layout.status.x + layout.status.w / 2,
            layout.status.y,
            layout.status.w / 2 - m.pad,
            layout.status.h,
        );
        self.text(right_text, right, p.text_muted, &self.formats.body_right.clone());
    }

    // -- pane --------------------------------------------------------------

    pub fn draw_pane(&mut self, m: crate::layout::Metrics, v: &PaneView) {
        if v.layout.bounds.is_empty() {
            return;
        }
        self.draw_tab_bar(m, v);
        self.draw_breadcrumb(m, v);
        self.draw_header(m, v);
        self.draw_rows(m, v);
        self.draw_scrollbar(v);

        // Focus ring around the whole content area of the active pane.
        if v.focused {
            let p = self.palette();
            let area = Rect::new(
                v.layout.list.x,
                v.layout.header.y,
                v.layout.list.w + v.layout.scrollbar.w,
                v.layout.list.h + v.layout.header.h,
            );
            self.stroke(area, p.focus, 1.0);
        }
    }

    fn draw_tab_bar(&mut self, m: crate::layout::Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.tab_bar, p.tab_bar_bg);

        for (i, t) in v.layout.tabs.clone().iter().enumerate() {
            let active = i == v.active_tab;
            if active {
                self.fill(t.full, p.tab_active);
                // Accent strip along the top of the active tab.
                let strip = Rect::new(t.full.x, t.full.y, t.full.w, 2.max(m.scale as i32));
                self.fill(strip, p.focus);
            } else if v.hover == Some(Hit::Tab(v.side, i)) || v.hover == Some(Hit::TabClose(v.side, i)) {
                self.fill(t.full, p.tab_hover);
            }

            let label_w = (t.close.w.max(0)).max(0);
            let label = Rect::new(
                t.full.x + m.pad,
                t.full.y,
                (t.full.w - m.pad - label_w).max(0),
                t.full.h,
            );
            let fg = if active { p.text } else { p.text_muted };
            let text = v.tab_labels.get(i).cloned().unwrap_or_default();
            self.text(&text, label, fg, &self.formats.small.clone());

            if !t.close.is_empty() {
                let hovered_close = v.hover == Some(Hit::TabClose(v.side, i));
                if hovered_close {
                    self.fill(t.close, p.row_hover);
                }
                let c = if hovered_close { p.text } else { p.text_muted };
                // Centred multiplication sign reads better than a lowercase x.
                self.text("\u{00D7}", t.close, c, &self.formats.small.clone());
            }

            // Separator between inactive tabs.
            if !active {
                let sep = Rect::new(t.full.right() - 1, t.full.y + m.pad / 2, 1, t.full.h - m.pad);
                self.fill(sep, p.divider);
            }
        }

        if !v.layout.new_tab.is_empty() {
            let hovered = v.hover == Some(Hit::NewTab(v.side));
            if hovered {
                self.fill(v.layout.new_tab, p.tab_hover);
            }
            let c = if hovered { p.text } else { p.text_muted };
            self.text("+", v.layout.new_tab, c, &self.formats.small.clone());
        }
    }

    fn draw_breadcrumb(&mut self, _m: crate::layout::Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.breadcrumb, p.pane_bg);

        let crumbs = v.layout.crumbs.clone();
        let last_index = crumbs.len().saturating_sub(1);
        for (i, c) in crumbs.iter().enumerate() {
            let hovered = v.hover == Some(Hit::Crumb(v.side, c.segment_index));
            if hovered {
                self.fill(c.rect, p.row_hover);
            }
            let label = if c.is_ellipsis {
                "\u{2026}".to_string()
            } else {
                v.crumbs.get(c.segment_index).cloned().unwrap_or_default()
            };
            let is_leaf = i == last_index && !c.is_ellipsis;
            let fg = if is_leaf || hovered { p.text } else { p.text_muted };
            let fmt = if is_leaf {
                self.formats.small_bold.clone()
            } else {
                self.formats.small.clone()
            };
            self.text(&label, c.rect, fg, &fmt);

            // Chevron between crumbs, drawn in the gap the layout left for it.
            if i < last_index {
                let gap = Rect::new(
                    c.rect.right(),
                    c.rect.y,
                    (crumbs[i + 1].rect.x - c.rect.right()).max(0),
                    c.rect.h,
                );
                self.text("\u{203A}", gap, p.text_muted, &self.formats.small.clone());
            }
        }

        let underline = Rect::new(
            v.layout.breadcrumb.x,
            v.layout.breadcrumb.bottom() - 1,
            v.layout.breadcrumb.w,
            1,
        );
        self.fill(underline, p.divider);
    }

    fn draw_header(&mut self, m: crate::layout::Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.header, p.header_bg);

        for c in v.layout.columns.clone() {
            let hovered = v.hover == Some(Hit::Column(v.side, c.key));
            if hovered {
                self.fill(c.rect, p.row_hover);
            }
            let sorted = v.list.sort_key == c.key;
            let title = match c.key {
                SortKey::Name => "Name",
                SortKey::Size => "Size",
                SortKey::Date => "Date modified",
            };
            let right_aligned = matches!(c.key, SortKey::Size);
            let fmt = if right_aligned {
                self.formats.body_right.clone()
            } else if sorted {
                self.formats.small_bold.clone()
            } else {
                self.formats.small.clone()
            };
            let inner = Rect::new(
                c.rect.x + m.pad,
                c.rect.y,
                (c.rect.w - m.pad * 2 - if sorted { m.pad + 4 } else { 0 }).max(0),
                c.rect.h,
            );
            let fg = if sorted { p.text } else { p.text_muted };
            self.text(title, inner, fg, &fmt);

            if sorted {
                let arrow = if v.list.sort_order == SortOrder::Asc {
                    "\u{25B2}"
                } else {
                    "\u{25BC}"
                };
                let ar = Rect::new(c.rect.right() - m.pad * 2, c.rect.y, m.pad * 2, c.rect.h);
                self.text(arrow, ar, p.text_muted, &self.formats.small.clone());
            }

            let sep = Rect::new(c.rect.right() - 1, c.rect.y + 4, 1, c.rect.h - 8);
            self.fill(sep, p.divider);
        }

        let underline = Rect::new(
            v.layout.header.x,
            v.layout.header.bottom() - 1,
            v.layout.header.w,
            1,
        );
        self.fill(underline, p.divider);
    }

    fn draw_rows(&mut self, m: crate::layout::Metrics, v: &PaneView) {
        let p = self.palette();
        let list_rect = v.layout.list;
        self.fill(list_rect, p.pane_bg);

        if let Some(err) = v.error {
            self.draw_centered_message(list_rect, &format!("Cannot open this folder\n{}", err), p.text_muted);
            return;
        }
        if v.loading && v.list.entries.is_empty() {
            self.draw_centered_message(list_rect, "Loading\u{2026}", p.text_muted);
            return;
        }
        if v.list.entries.is_empty() {
            self.draw_centered_message(list_rect, "This folder is empty", p.text_muted);
            return;
        }

        self.push_clip(list_rect);

        let row_h = m.row_h;
        let (start, end) = v.list.visible_range(list_rect.h as u32);
        let cursor = v.list.cursor();

        // Column geometry comes from the same layout the header used, so the
        // cells line up with their titles by construction.
        let cols = v.layout.columns.clone();
        let name_col = cols.iter().find(|c| c.key == SortKey::Name).map(|c| c.rect);
        let size_col = cols.iter().find(|c| c.key == SortKey::Size).map(|c| c.rect);
        let date_col = cols.iter().find(|c| c.key == SortKey::Date).map(|c| c.rect);

        for row in start..end {
            let Some(entry) = v.list.entries.get(row as usize) else {
                break;
            };
            let y = list_rect.y + ((row - v.list.scroll_offset) as i32) * row_h;
            if y >= list_rect.bottom() {
                break;
            }
            let r = Rect::new(list_rect.x, y, list_rect.w, row_h);

            let selected = v.list.is_selected(row);
            let hovered = v.hover == Some(Hit::Row(v.side, row));

            if row % 2 == 1 {
                self.fill(r, p.row_alt);
            }
            if selected {
                let c = if v.focused { p.selection } else { p.selection_inactive };
                self.fill(r, c);
            } else if hovered {
                self.fill(r, p.row_hover);
            }
            if Some(row) == cursor && v.focused {
                self.stroke(r, p.cursor_outline, 1.0);
            }

            let fg = if selected { p.text_on_selection } else { p.text };
            let muted = if selected { p.text_on_selection } else { p.text_muted };

            // Name cell: icon then text.
            if let Some(nc) = name_col {
                let icon = Rect::new(
                    nc.x + m.pad,
                    y + (row_h - m.icon_size) / 2,
                    m.icon_size,
                    m.icon_size,
                );
                let key = icon_key(entry.extension.as_deref(), entry.is_dir);
                self.draw_icon(&key, icon);

                let name = Rect::new(
                    icon.right() + m.pad,
                    y,
                    (nc.right() - icon.right() - m.pad * 2).max(0),
                    row_h,
                );
                self.text(&entry.name, name, fg, &self.formats.body.clone());

                // Junctions and symlinks are marked so a recursive copy is not a
                // surprise.
                if entry.is_reparse {
                    let badge = Rect::new(icon.x, icon.bottom() - 6, 6, 6);
                    self.fill(badge, p.focus);
                }
            }

            if let Some(sc) = size_col {
                let cell = Rect::new(sc.x, y, sc.w - m.pad, row_h);
                self.text(&entry.size_display(), cell, muted, &self.formats.body_right.clone());
            }
            if let Some(dc) = date_col {
                let cell = Rect::new(dc.x + m.pad, y, dc.w - m.pad * 2, row_h);
                self.text(&entry.date_display(), cell, muted, &self.formats.body.clone());
            }
        }

        self.pop_clip();
    }

    fn draw_centered_message(&mut self, area: Rect, msg: &str, c: Rgb) {
        let h = 40;
        let r = Rect::new(area.x, area.y + (area.h - h) / 2, area.w, h);
        let fmt = self.formats.small.clone();
        // Centre horizontally by borrowing a centred copy of the format.
        unsafe {
            let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
        }
        self.text(msg, r, c, &fmt);
        unsafe {
            let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        }
    }

    fn draw_scrollbar(&mut self, v: &PaneView) {
        let p = self.palette();
        if v.layout.scrollbar.is_empty() {
            return;
        }
        self.fill(v.layout.scrollbar, p.pane_bg);
        if v.layout.thumb.is_empty() {
            return;
        }
        let hovered = matches!(v.hover, Some(Hit::ScrollbarThumb(s)) if s == v.side);
        let c = if hovered {
            p.scrollbar_thumb_hover
        } else {
            p.scrollbar_thumb
        };
        self.fill(v.layout.thumb.inset(2, 2), c);
    }
}

impl TextMeasurer for Renderer {
    fn measure(&self, text: &str, small: bool) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let key = (text.to_string(), small);
        if let Some(w) = self.measure_cache.borrow().get(&key) {
            return *w;
        }
        let fmt = if small {
            &self.formats.small
        } else {
            &self.formats.body
        };
        let buf: Vec<u16> = text.encode_utf16().collect();
        let width = unsafe {
            match self
                .dwrite
                .CreateTextLayout(&buf, fmt, f32::MAX / 4.0, f32::MAX / 4.0)
            {
                Ok(layout) => {
                    let mut metrics = DWRITE_TEXT_METRICS::default();
                    if layout.GetMetrics(&mut metrics).is_ok() {
                        metrics.width
                    } else {
                        text.chars().count() as f32 * 7.0
                    }
                }
                // Fall back to a rough estimate rather than collapsing to zero,
                // which would make every tab and crumb the minimum width.
                Err(_) => text.chars().count() as f32 * 7.0,
            }
        };
        self.measure_cache.borrow_mut().insert(key, width);
        width
    }
}
