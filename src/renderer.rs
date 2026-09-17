// Direct2D / DirectWrite painting.
//
// Departures from the original renderer, all deliberate:
//
//  * One reusable solid brush whose colour is set per draw, instead of eleven
//    cached brushes torn down and rebuilt on every WM_SIZE.
//  * `Resize` on the existing render target rather than destroying and
//    recreating it, so dragging the window edge does not thrash the GPU.
//  * Text formats carry no-wrap, ellipsis trimming, and vertical centring, so a
//    long filename is trimmed with an ellipsis instead of wrapping inside a
//    row and being clipped mid-glyph.
//  * Chrome glyphs (chevrons, close, plus, sort arrows) come from Segoe MDL2
//    Assets rather than being approximated with punctuation.
//
// The render target runs at 96 DPI so one DIP is one physical pixel; scaling
// for the monitor happens once, in `layout::Metrics`, and in the font sizes
// below. That keeps every coordinate in the app in the same unit.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use windows::core::{Interface, Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::Graphics::Gdi::{HBITMAP, HPALETTE};
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::file_list::{FileList, SortKey, SortOrder};
use crate::fs::Drive;
use crate::icons::{icon_key, icon_key_for_path, IconCache};
use crate::layout::{
    sidebar_text_offset, Hit, Layout, Metrics, NavButton, PaneLayout, Rect, SidebarEntry, PaneId,
    TextMeasurer,
};
use crate::theme::{Palette, Rgb, Theme};

// Segoe MDL2 Assets code points. Present on every Windows 10 and 11 install.
pub mod glyph {
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const UP: &str = "\u{E74A}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const ADD: &str = "\u{E710}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const FILTER: &str = "\u{E71C}";

    // The command bar.
    pub const CUT: &str = "\u{E8C6}";
    pub const COPY: &str = "\u{E8C8}";
    pub const PASTE: &str = "\u{E77F}";
    pub const RENAME: &str = "\u{E8AC}";
    pub const DELETE: &str = "\u{E74D}";
    pub const SORT: &str = "\u{E8CB}";
    pub const VIEW: &str = "\u{E890}";
    pub const MORE: &str = "\u{E712}";
    pub const PANE: &str = "\u{E8A0}";
    pub const SHARE: &str = "\u{E72D}";
}

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
    pub pid: PaneId,
    pub layout: &'a PaneLayout,
    pub tab_labels: &'a [String],
    pub active_tab: usize,
    pub crumbs: &'a [String],
    pub list: &'a FileList,
    pub focused: bool,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub hover: Option<Hit>,
    pub can_back: bool,
    pub can_forward: bool,
    pub can_up: bool,
    /// True when keystrokes are going to the filter box rather than the list.
    pub filter_focused: bool,
    /// The rubber-band rectangle being dragged in this pane, already clipped
    /// to the rows area.
    pub band: Option<Rect>,
    /// The row whose name is being edited in place, if it is in this pane.
    pub renaming: Option<u32>,
    /// Shell images for the icon view, and this pane's folder, which together
    /// give each entry its cache key. Empty in the details view.
    pub thumbs: Option<(&'a crate::preview::ThumbCache, &'a str)>,
    pub counts: &'a str,
    /// In compare mode, the names present in the *other* pane. Rows missing
    /// from it get a marker, which is what makes two trees diffable at a glance.
    pub other_names: Option<&'a HashSet<String>>,
}

/// Everything the renderer needs to paint the inspector.
pub struct InspectorView<'a> {
    /// None when nothing is under the cursor.
    pub name: Option<&'a str>,
    pub kind: String,
    pub size: String,
    pub modified: String,
    /// The preview and the key it was built for; the key is what the converted
    /// bitmap is cached against.
    pub preview: Option<(&'a str, &'a crate::preview::Preview)>,
}

/// One command-bar button, ready to draw.
pub struct BarButtonView<'a> {
    pub glyph: &'a str,
    pub label: &'a str,
    pub menu: bool,
    pub enabled: bool,
}

/// Everything the renderer needs to paint the sidebar.
pub struct SidebarView<'a> {
    pub entries: &'a [SidebarEntry],
    pub drives: &'a [Drive],
    /// (label, path) shortcuts.
    pub places: &'a [(String, String)],
    pub current_path: &'a str,
    /// Visible rows of the folder tree, indexed by `SidebarEntry::Tree`.
    pub tree: &'a [crate::tree::TreeRow],
    pub hover: Option<Hit>,
}

struct Formats {
    body: IDWriteTextFormat,
    body_right: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_bold: IDWriteTextFormat,
    small_right: IDWriteTextFormat,
    tiny: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    centered: IDWriteTextFormat,
    /// Centred and wrapping to two lines, for a name under an icon.
    cell_label: IDWriteTextFormat,
    /// Wrapping, top-aligned, no ellipsis: for the inspector's text preview,
    /// which is many lines rather than one that must fit.
    preview: IDWriteTextFormat,
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
    /// The inspector shows one preview at a time, so its converted bitmap is
    /// one slot keyed by the preview it came from rather than a cache.
    preview_bitmap: Option<(String, ID2D1Bitmap)>,
    /// Converted grid-cell images. Keyed like the cache they come from; the
    /// cache is what bounds this, since a key it has evicted is never asked
    /// for again and `prune_cells` drops what it no longer holds.
    cell_bitmaps: HashMap<String, ID2D1Bitmap>,
    /// Memo for text measurement: layout asks for the same tab and crumb
    /// widths every frame.
    measure_cache: RefCell<HashMap<(String, bool), f32>>,
}

impl Renderer {
    pub fn new(theme: Theme, dpi: u32) -> Result<Self> {
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
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
            preview_bitmap: None,
            cell_bitmaps: HashMap::new(),
            measure_cache: RefCell::new(HashMap::new()),
        })
    }

    fn make_formats(dwrite: &IDWriteFactory, dpi: u32) -> Result<Formats> {
        let scale = dpi as f32 / 96.0;
        let ui = wide("Segoe UI");
        let mdl2 = wide("Segoe MDL2 Assets");
        let locale = wide("en-us");

        let mk = |family: &[u16],
                  size: f32,
                  weight: DWRITE_FONT_WEIGHT,
                  align: DWRITE_TEXT_ALIGNMENT,
                  ellipsis: bool|
         -> Result<IDWriteTextFormat> {
            let f = unsafe {
                dwrite.CreateTextFormat(
                    PCWSTR::from_raw(family.as_ptr()),
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
                if ellipsis {
                    let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
                    let trimming = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    };
                    f.SetTrimming(&trimming, &sign)?;
                }
            }
            Ok(f)
        };

        use DWRITE_FONT_WEIGHT_NORMAL as NORMAL;
        use DWRITE_FONT_WEIGHT_SEMI_BOLD as SEMI;
        use DWRITE_TEXT_ALIGNMENT_CENTER as CENTER;
        use DWRITE_TEXT_ALIGNMENT_LEADING as LEADING;
        use DWRITE_TEXT_ALIGNMENT_TRAILING as TRAILING;

        Ok(Formats {
            body: mk(&ui, 12.5, NORMAL, LEADING, true)?,
            body_right: mk(&ui, 12.5, NORMAL, TRAILING, true)?,
            small: mk(&ui, 12.0, NORMAL, LEADING, true)?,
            small_bold: mk(&ui, 12.0, SEMI, LEADING, true)?,
            small_right: mk(&ui, 12.0, NORMAL, TRAILING, true)?,
            tiny: mk(&ui, 10.5, NORMAL, LEADING, true)?,
            caption: mk(&ui, 10.5, SEMI, LEADING, true)?,
            icon: mk(&mdl2, 10.0, NORMAL, CENTER, false)?,
            centered: mk(&ui, 12.5, NORMAL, CENTER, false)?,
            cell_label: {
                let f = mk(&ui, 11.5, NORMAL, CENTER, true)?;
                unsafe {
                    f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
                    f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                }
                f
            },
            preview: {
                let f = mk(&ui, 11.5, NORMAL, LEADING, false)?;
                unsafe {
                    // Every other format is one line in a row: top-aligned and
                    // wrapping are both wrong there and both right here.
                    f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
                    f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                }
                f
            },
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
        let brush = unsafe { target.CreateSolidColorBrush(&color((1.0, 1.0, 1.0)), None)? };
        self.brush = Some(brush);
        self.target = Some(target);
        self.icon_bitmaps.clear();
        self.preview_bitmap = None;
        self.cell_bitmaps.clear();
        Ok(())
    }

    /// Drop the device-dependent resources after D2DERR_RECREATE_TARGET.
    pub fn discard_target(&mut self) {
        self.target = None;
        self.brush = None;
        self.icon_bitmaps.clear();
        self.preview_bitmap = None;
        self.cell_bitmaps.clear();
    }

    pub fn has_target(&self) -> bool {
        self.target.is_some()
    }

    // -- primitives --------------------------------------------------------

    fn rt(&self) -> Option<&ID2D1HwndRenderTarget> {
        self.target.as_ref()
    }

    /// A hollow rectangle, drawn as four fills. D2D can stroke one, but that
    /// wants a stroke style and half-pixel alignment to come out crisp; four
    /// rectangles are exact at any DPI.
    fn outline(&self, r: Rect, c: Rgb, t: i32) {
        if r.is_empty() || t <= 0 {
            return;
        }
        self.fill(Rect::new(r.x, r.y, r.w, t), c);
        self.fill(Rect::new(r.x, r.bottom() - t, r.w, t), c);
        self.fill(Rect::new(r.x, r.y, t, r.h), c);
        self.fill(Rect::new(r.right() - t, r.y, t, r.h), c);
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

    fn fill_rounded(&self, r: Rect, radius: f32, c: Rgb) {
        if r.is_empty() {
            return;
        }
        let (Some(rt), Some(brush)) = (self.rt(), self.brush.as_ref()) else {
            return;
        };
        // Never round more than half the shorter pid, or the shape inverts.
        let radius = radius.min(r.w as f32 / 2.0).min(r.h as f32 / 2.0).max(0.0);
        let rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(r),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            brush.SetColor(&color(c));
            rt.FillRoundedRectangle(&rr, brush);
        }
    }

    fn stroke_rounded(&self, r: Rect, radius: f32, c: Rgb, width: f32) {
        if r.is_empty() {
            return;
        }
        let (Some(rt), Some(brush)) = (self.rt(), self.brush.as_ref()) else {
            return;
        };
        let half = width / 2.0;
        let inner = Rect::new(
            r.x + half as i32,
            r.y + half as i32,
            r.w - width as i32,
            r.h - width as i32,
        );
        let radius = radius.min(inner.w as f32 / 2.0).min(inner.h as f32 / 2.0).max(0.0);
        let rr = D2D1_ROUNDED_RECT {
            rect: d2d_rect(inner),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            brush.SetColor(&color(c));
            rt.DrawRoundedRectangle(&rr, brush, width, None);
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

    fn glyph(&self, g: &str, r: Rect, c: Rgb) {
        self.text(g, r, c, &self.formats.icon.clone());
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

    /// The preview image as a D2D bitmap, converted once and kept until the
    /// selection moves on.
    fn preview_bitmap(&mut self, key: &str, hbm: HBITMAP) -> Option<ID2D1Bitmap> {
        if self.preview_bitmap.as_ref().is_some_and(|(k, _)| k == key) {
            return self.preview_bitmap.as_ref().map(|(_, b)| b.clone());
        }
        let bmp = self.hbitmap_to_bitmap(hbm).ok()?;
        self.preview_bitmap = Some((key.to_string(), bmp.clone()));
        Some(bmp)
    }

    /// A shell thumbnail is an HBITMAP, not an HICON, so it takes the other WIC
    /// entry point. The rest of the pipeline is the icons' one.
    fn hbitmap_to_bitmap(&self, hbm: HBITMAP) -> Result<ID2D1Bitmap> {
        let rt = self
            .rt()
            .ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?;
        unsafe {
            let src = self.wic.CreateBitmapFromHBITMAP(
                hbm,
                HPALETTE::default(),
                // Shell thumbnails carry alpha for anything with transparency;
                // ignoring it paints black behind a PNG.
                WICBitmapUsePremultipliedAlpha,
            )?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(
                &src,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            Ok(rt.CreateBitmapFromWicBitmap(&converter, None)?.cast()?)
        }
    }

    /// The command bar: a row of buttons over the whole window, acting on
    /// whichever pane has focus.
    pub fn draw_command_bar(&mut self, layout: &Layout, items: &[BarButtonView], hover: Option<Hit>) {
        if layout.bar.is_empty() {
            return;
        }
        let p = self.palette();
        let m = layout.metrics;
        self.fill(layout.bar, p.toolbar_bg);
        // A hairline underneath, so the bar reads as chrome rather than as the
        // top of the first pane.
        let edge = Rect::new(
            layout.bar.x,
            layout.bar.bottom() - 1.max(m.scale as i32),
            layout.bar.w,
            1.max(m.scale as i32),
        );
        self.fill(edge, p.divider);

        for (i, item) in items.iter().enumerate() {
            let Some(r) = layout.bar_items.get(i).copied() else {
                break;
            };
            if r.is_empty() {
                continue;
            }
            if item.enabled && hover == Some(Hit::Bar(i)) {
                self.fill_rounded(r, m.radius, p.row_hover);
            }
            let fg = if item.enabled { p.text } else { p.text_faint };

            // Glyph first, then the label, then the chevron — laid out left to
            // right so an icon-only button centres its glyph in the whole
            // button rather than in a notional icon column.
            let glyph_w = if item.label.is_empty() { r.w } else { m.bar_btn_w };
            self.glyph(item.glyph, Rect::new(r.x, r.y, glyph_w, r.h), fg);
            if !item.label.is_empty() {
                let chevron = if item.menu { m.pad * 2 } else { 0 };
                let text = Rect::new(
                    r.x + glyph_w,
                    r.y,
                    (r.w - glyph_w - chevron).max(0),
                    r.h,
                );
                self.text(item.label, text, fg, &self.formats.small.clone());
                if item.menu {
                    self.glyph(
                        glyph::CHEVRON_DOWN,
                        Rect::new(r.right() - chevron, r.y, chevron, r.h),
                        fg,
                    );
                }
            }
        }
    }

    /// The inspector panel: what one file is, and as much of it as can be shown
    /// without opening it.
    pub fn draw_inspector(&mut self, layout: &Layout, v: &InspectorView) {
        let r = layout.inspector;
        if r.is_empty() {
            return;
        }
        let p = self.palette();
        let m = layout.metrics;
        self.fill(r, p.sidebar_bg);
        self.fill(Rect::new(r.x, r.y, 1.max(m.scale as i32), r.h), p.divider);

        let Some(name) = v.name else {
            self.centered_message(r, "Nothing selected");
            return;
        };

        self.push_clip(r);
        let inner = Rect::new(r.x + m.pad * 2, r.y + m.pad, r.w - m.pad * 3, r.h - m.pad * 2);
        let line = m.row_h;

        // Name first, then the facts, then whatever the file itself can show.
        let mut y = inner.y;
        self.text(
            name,
            Rect::new(inner.x, y, inner.w, line),
            p.text,
            &self.formats.small_bold.clone(),
        );
        y += line;
        for fact in [&v.kind, &v.size, &v.modified] {
            if fact.is_empty() {
                continue;
            }
            self.text(
                fact,
                Rect::new(inner.x, y, inner.w, line),
                p.text_muted,
                &self.formats.tiny.clone(),
            );
            y += line * 3 / 4;
        }
        y += m.pad;

        let body = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(0));
        if body.h <= line {
            self.pop_clip();
            return;
        }
        let faint = |r: &mut Self, msg: &str| {
            r.text(
                msg,
                Rect::new(body.x, body.y, body.w, line),
                p.text_faint,
                &r.formats.tiny.clone(),
            )
        };
        match v.preview {
            Some((key, crate::preview::Preview::Image(hbm))) => {
                self.draw_preview_image(key, *hbm, body)
            }
            Some((_, crate::preview::Preview::Text(text))) => {
                self.text(text, body, p.text_muted, &self.formats.preview.clone())
            }
            // Peeking into a folder: the names, and how many did not fit.
            Some((_, crate::preview::Preview::Folder { names, more })) => {
                let mut listing = names.join("\r\n");
                if *more > 0 {
                    listing.push_str(&format!("\r\n\u{2026} and {} more", more));
                }
                self.text(&listing, body, p.text_muted, &self.formats.preview.clone())
            }
            Some((_, crate::preview::Preview::None)) => {
                faint(self, "No preview for this kind of file.")
            }
            // Nothing has come back from the worker yet.
            None => faint(self, "Reading\u{2026}"),
        }
        self.pop_clip();
    }

    /// Fit the thumbnail inside `area` without stretching it, pinned to the
    /// top: a portrait photo and a landscape one should start at the same
    /// place, and neither should be distorted to fill the panel.
    fn draw_preview_image(&mut self, key: &str, hbm: HBITMAP, area: Rect) {
        let Some(bmp) = self.preview_bitmap(key, hbm) else {
            return;
        };
        let size = unsafe { bmp.GetSize() };
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let scale = (area.w as f32 / size.width)
            .min(area.h as f32 / size.height)
            // Never blow a 48px thumbnail up to panel width: a smeared image
            // reads as a broken one.
            .min(1.0);
        let w = (size.width * scale).round() as i32;
        let h = (size.height * scale).round() as i32;
        let dest = Rect::new(area.x + (area.w - w) / 2, area.y, w, h);
        if let Some(rt) = self.rt() {
            unsafe {
                rt.DrawBitmap(
                    &bmp,
                    Some(&d2d_rect(dest)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
    }

    /// Draw a grid cell's shell image, or say it is not there yet.
    ///
    /// Returns false when there is nothing to draw, which is the caller's cue
    /// to fall back to the small file-type icon: something in every cell from
    /// the first frame, replaced as the real images arrive.
    fn draw_cell_image(&mut self, key: &str, cache: &crate::preview::ThumbCache, area: Rect) -> bool {
        if !self.cell_bitmaps.contains_key(key) {
            let Some(hbm) = cache.get(key) else {
                return false;
            };
            let Ok(bmp) = self.hbitmap_to_bitmap(hbm) else {
                return false;
            };
            self.cell_bitmaps.insert(key.to_string(), bmp);
        }
        let Some(bmp) = self.cell_bitmaps.get(key).cloned() else {
            return false;
        };
        let size = unsafe { bmp.GetSize() };
        if size.width <= 0.0 || size.height <= 0.0 {
            return false;
        }
        // Fit inside the cell's icon box, centred, never enlarged past it.
        let scale = (area.w as f32 / size.width).min(area.h as f32 / size.height);
        let w = (size.width * scale).round() as i32;
        let h = (size.height * scale).round() as i32;
        let dest = Rect::new(
            area.x + (area.w - w) / 2,
            area.y + (area.h - h) / 2,
            w,
            h,
        );
        if let Some(rt) = self.rt() {
            unsafe {
                rt.DrawBitmap(
                    &bmp,
                    Some(&d2d_rect(dest)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
        true
    }

    /// Drop converted bitmaps whose cache entry has been evicted, so this map
    /// cannot outgrow the cache that feeds it.
    pub fn prune_cells(&mut self, cache: &crate::preview::ThumbCache) {
        self.cell_bitmaps.retain(|k, _| cache.has(k));
    }

    fn draw_icon(&mut self, key: &str, r: Rect) {
        let dest = d2d_rect(r);
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

    /// Finish the frame. Err means the device was lost and the caller should
    /// discard the target and repaint.
    pub fn end(&self) -> Result<()> {
        if let Some(rt) = self.rt() {
            unsafe { rt.EndDraw(None, None)? };
        }
        Ok(())
    }

    // -- sidebar -----------------------------------------------------------

    pub fn draw_sidebar(&mut self, layout: &Layout, v: &SidebarView) {
        if layout.sidebar.is_empty() {
            return;
        }
        let p = self.palette();
        let m = layout.metrics;
        self.fill(layout.sidebar, p.sidebar_bg);

        self.push_clip(layout.sidebar);
        let rows = layout.sidebar_rows.clone();
        for (i, row) in rows.iter().enumerate() {
            if row.rect.is_empty() {
                continue;
            }
            let Some(entry) = v.entries.get(i) else { continue };
            match entry {
                SidebarEntry::Section { label, collapsed } => {
                    let hovered = matches!(
                        v.hover,
                        Some(Hit::Sidebar(h)) | Some(Hit::SidebarChevron(h)) if h == i
                    );
                    let text_rect = Rect::new(
                        row.rect.x + m.pad,
                        row.rect.y,
                        row.rect.w - m.pad * 2 - m.icon_size,
                        row.rect.h,
                    );
                    self.text(
                        &label.to_uppercase(),
                        text_rect,
                        if hovered { p.text_muted } else { p.text_faint },
                        &self.formats.caption.clone(),
                    );
                    let g = if *collapsed {
                        glyph::CHEVRON_RIGHT
                    } else {
                        glyph::CHEVRON_DOWN
                    };
                    self.glyph(g, row.chevron, if hovered { p.text } else { p.text_faint });
                }

                SidebarEntry::Drive { index, used } => {
                    let Some(d) = v.drives.get(*index) else { continue };
                    let active = v
                        .current_path
                        .to_lowercase()
                        .starts_with(&d.root.to_lowercase());
                    let key = icon_key_for_path(&d.root);
                    let label = d.display();
                    let sub = used.map(|_| d.capacity_text());
                    self.draw_sidebar_row(m, row.rect, i, v.hover, active, &key, &label, sub);
                    if let Some(frac) = used {
                        self.draw_capacity_bar(row.capacity, *frac);
                    }
                }

                SidebarEntry::Tree {
                    index,
                    depth,
                    expanded,
                } => {
                    let Some(tr) = v.tree.get(*index) else { continue };
                    let active = crate::pane::paths_equal(v.current_path, &tr.path);
                    self.draw_tree_row(
                        m,
                        row.rect,
                        i,
                        v.hover,
                        active,
                        &tr.label,
                        *depth,
                        *expanded,
                    );
                }

                SidebarEntry::Place { index } => {
                    let Some((label, path)) = v.places.get(*index) else {
                        continue;
                    };
                    let active = crate::pane::paths_equal(v.current_path, path);
                    let key = icon_key_for_path(path);
                    let label = label.clone();
                    self.draw_sidebar_row(m, row.rect, i, v.hover, active, &key, &label, None);
                }
            }
        }
        self.pop_clip();

        // Hairline between the sidebar and the panes.
        let edge = Rect::new(layout.sidebar.right() - 1, layout.sidebar.y, 1, layout.sidebar.h);
        self.fill(edge, p.divider);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sidebar_row(
        &mut self,
        m: Metrics,
        rect: Rect,
        index: usize,
        hover: Option<Hit>,
        active: bool,
        icon_cache_key: &str,
        label: &str,
        subtitle: Option<String>,
    ) {
        let p = self.palette();
        let hovered = hover == Some(Hit::Sidebar(index));
        let pill = Rect::new(rect.x + m.pad / 2, rect.y, rect.w - m.pad, rect.h);

        if active {
            self.fill_rounded(pill, m.radius, p.selection_inactive);
            // Accent stub marking the volume the focused pane is inside.
            let bar = Rect::new(rect.x, rect.y + 3, 2.max(m.scale as i32), rect.h - 6);
            self.fill(bar, p.accent);
        } else if hovered {
            self.fill_rounded(pill, m.radius, p.row_hover);
        }

        // Two-line rows hang the icon off the first line; one-line rows centre it.
        let icon_y = match subtitle {
            Some(_) => rect.y + m.pad / 2 + (m.sidebar_row_h - m.icon_size) / 2 - m.pad / 4,
            None => rect.y + (rect.h - m.icon_size) / 2,
        };
        let icon = Rect::new(rect.x + m.pad + m.pad / 2, icon_y, m.icon_size, m.icon_size);
        self.draw_icon(icon_cache_key, icon);

        let text_x = rect.x + sidebar_text_offset(m);
        let text_w = (pill.right() - text_x - m.pad / 2).max(0);
        match subtitle {
            Some(sub) => {
                let top = Rect::new(text_x, rect.y + m.pad / 4, text_w, m.sidebar_row_h);
                self.text(label, top, p.text, &self.formats.small.clone());
                if !sub.is_empty() {
                    let bottom = Rect::new(
                        text_x,
                        top.bottom() - m.pad / 2,
                        text_w,
                        (rect.bottom() - top.bottom()).max(0),
                    );
                    self.text(&sub, bottom, p.text_faint, &self.formats.tiny.clone());
                }
            }
            None => {
                let r = Rect::new(text_x, rect.y, text_w, rect.h);
                self.text(label, r, p.text, &self.formats.small.clone());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tree_row(
        &mut self,
        m: Metrics,
        rect: Rect,
        index: usize,
        hover: Option<Hit>,
        active: bool,
        label: &str,
        depth: usize,
        expanded: bool,
    ) {
        let p = self.palette();
        let hovered = matches!(
            hover,
            Some(Hit::Sidebar(h)) | Some(Hit::SidebarChevron(h)) if h == index
        );
        let pill = Rect::new(rect.x + m.pad / 2, rect.y, rect.w - m.pad, rect.h);
        if active {
            self.fill_rounded(pill, m.radius, p.selection_inactive);
        } else if hovered {
            self.fill_rounded(pill, m.radius, p.row_hover);
        }

        let indent = crate::layout::tree_indent(m, depth);
        let chevron = Rect::new(rect.x + m.pad / 2 + indent, rect.y, m.icon_size, rect.h);
        self.glyph(
            if expanded {
                glyph::CHEVRON_DOWN
            } else {
                glyph::CHEVRON_RIGHT
            },
            chevron,
            if hovered { p.text } else { p.text_faint },
        );

        let icon = Rect::new(
            chevron.right() + m.pad / 4,
            rect.y + (rect.h - m.icon_size) / 2,
            m.icon_size,
            m.icon_size,
        );
        self.draw_icon(&icon_key(None, true), icon);

        let text_x = icon.right() + m.pad / 2;
        let text = Rect::new(text_x, rect.y, (pill.right() - text_x - m.pad / 2).max(0), rect.h);
        self.text(label, text, p.text, &self.formats.small.clone());
    }

    fn draw_capacity_bar(&mut self, track: Rect, used: f32) {
        if track.is_empty() {
            return;
        }
        let p = self.palette();
        let radius = track.h as f32 / 2.0;
        self.fill_rounded(track, radius, p.capacity_track);
        let w = ((track.w as f32) * used.clamp(0.0, 1.0)).round() as i32;
        if w <= 0 {
            return;
        }
        // A nearly full disk is information, not decoration: colour it.
        let c = if used >= 0.9 {
            p.capacity_full
        } else {
            p.capacity_fill
        };
        self.fill_rounded(Rect::new(track.x, track.y, w.max(2), track.h), radius, c);
    }

    // -- divider -----------------------------------------------------------

    /// `active` names the divider under the cursor or being dragged, if any.
    pub fn draw_dividers(&mut self, layout: &Layout, active: Option<(usize, bool)>) {
        let p = self.palette();
        for (i, d) in layout.dividers.clone().iter().enumerate() {
            let r = d.rect;
            self.fill(r, p.window_bg);
            let c = match active {
                Some((n, true)) if n == i => p.accent,
                Some((n, false)) if n == i => p.scrollbar_thumb_hover,
                _ => p.divider,
            };
            // A hairline down the middle of the grab strip, along whichever
            // way the divider runs.
            let mid = if d.vertical {
                Rect::new(r.x + r.w / 2, r.y, 1.max(r.w / 6), r.h)
            } else {
                Rect::new(r.x, r.y + r.h / 2, r.w, 1.max(r.h / 6))
            };
            self.fill(mid, c);
        }
    }

    // -- pane --------------------------------------------------------------

    pub fn draw_pane(&mut self, m: Metrics, v: &PaneView) {
        if v.layout.bounds.is_empty() {
            return;
        }
        let p = self.palette();
        self.fill(v.layout.bounds, p.pane_bg);

        self.draw_tab_bar(m, v);
        self.draw_toolbar(m, v);
        self.draw_header(m, v);
        self.draw_rows(m, v);
        if let Some(band) = v.band {
            self.outline(band, p.accent, 1.max(m.scale as i32));
        }
        self.draw_scrollbar(v);
        self.draw_footer(m, v);

        // The focused pane is marked by an accent rule under its tab bar rather
        // than a ring around the content: it reads at a glance and costs no space.
        if v.focused {
            let bar = Rect::new(
                v.layout.toolbar.x,
                v.layout.toolbar.y,
                v.layout.toolbar.w,
                2.max(m.scale as i32),
            );
            self.fill(bar, p.accent);
        }
    }

    fn draw_tab_bar(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.tab_bar, p.tab_bar_bg);

        for (i, t) in v.layout.tabs.clone().iter().enumerate() {
            let active = i == v.active_tab;
            let hovered = matches!(v.hover, Some(Hit::Tab(s, h)) | Some(Hit::TabClose(s, h)) if s == v.pid && h == i);

            // The active tab is a card that sits on top of the toolbar below
            // it; the inactive ones are flat. That, rather than a slightly
            // different grey, is what makes the active one obvious at a glance.
            let card = Rect::new(t.full.x + 1, t.full.y + 3, t.full.w - 2, t.full.h - 3);
            if active {
                self.fill_rounded(card, m.radius * 1.5, p.tab_active);
                // Square off the bottom so the card joins the toolbar instead
                // of floating above it.
                self.fill(
                    Rect::new(card.x, card.bottom() - 6, card.w, 6),
                    p.tab_active,
                );
                // Accent bar: which pane you are typing into, at a glance.
                let bar_w = (card.w - m.pad * 2).max(8);
                self.fill_rounded(
                    Rect::new(card.x + (card.w - bar_w) / 2, card.y, bar_w, 2.max(m.scale as i32)),
                    1.0,
                    if v.focused { p.accent } else { p.text_faint },
                );
            } else if hovered {
                self.fill_rounded(card, m.radius * 1.5, p.tab_hover);
            }

            let icon = Rect::new(
                t.full.x + m.pad,
                t.full.y + (t.full.h - m.icon_size) / 2,
                m.icon_size,
                m.icon_size,
            );
            if t.full.w > m.icon_size + m.pad * 2 {
                self.draw_icon("<dir>", icon);
            }

            let label_x = icon.right() + m.pad / 2;
            let label_right = if t.close.is_empty() {
                t.full.right() - m.pad / 2
            } else {
                t.close.x
            };
            let label = Rect::new(label_x, t.full.y, (label_right - label_x).max(0), t.full.h);
            let fg = if active { p.text } else { p.text_muted };
            let text = v.tab_labels.get(i).cloned().unwrap_or_default();
            self.text(&text, label, fg, &self.formats.small.clone());

            if !t.close.is_empty() {
                let hovered_close = v.hover == Some(Hit::TabClose(v.pid, i));
                if hovered_close {
                    self.fill_rounded(t.close, m.radius, p.row_hover);
                }
                let c = if hovered_close { p.text } else { p.text_faint };
                self.glyph(glyph::CLOSE, t.close, c);
            }

            // A hairline between two inactive tabs only. Beside the active
            // card it would cut into its rounded corner.
            let next_active = i + 1 == v.active_tab;
            if !active && !next_active && i + 1 < v.layout.tabs.len() {
                let sep = Rect::new(t.full.right() - 1, t.full.y + 9, 1, t.full.h - 18);
                self.fill(sep, p.divider);
            }
        }

        if !v.layout.new_tab.is_empty() {
            let hovered = v.hover == Some(Hit::NewTab(v.pid));
            if hovered {
                self.fill_rounded(v.layout.new_tab.inset(3, 5), m.radius, p.tab_hover);
            }
            let c = if hovered { p.text } else { p.text_faint };
            self.glyph(glyph::ADD, v.layout.new_tab, c);
        }

        // A hairline under the whole strip, broken where the active card meets
        // it, so the card reads as continuous with the toolbar.
        let under = Rect::new(
            v.layout.tab_bar.x,
            v.layout.tab_bar.bottom() - 1,
            v.layout.tab_bar.w,
            1,
        );
        self.fill(under, p.divider);
        if let Some(t) = v.layout.tabs.get(v.active_tab) {
            self.fill(
                Rect::new(t.full.x + 1, under.y, (t.full.w - 2).max(0), 1),
                p.tab_active,
            );
        }
    }

    fn draw_toolbar(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.toolbar, p.toolbar_bg);

        let buttons = [
            (v.layout.nav_back, NavButton::Back, glyph::BACK, v.can_back),
            (
                v.layout.nav_forward,
                NavButton::Forward,
                glyph::FORWARD,
                v.can_forward,
            ),
            (v.layout.nav_up, NavButton::Up, glyph::UP, v.can_up),
        ];
        for (rect, which, g, enabled) in buttons {
            let hovered = v.hover == Some(Hit::Nav(v.pid, which));
            if hovered && enabled {
                self.fill_rounded(rect.inset(2, 4), m.radius, p.row_hover);
            }
            // A disabled control that looks enabled is worse than no control.
            let c = if !enabled {
                p.text_faint
            } else if hovered {
                p.text
            } else {
                p.text_muted
            };
            self.glyph(g, rect, c);
        }

        // Belt and braces: the layout already fits the crumbs, and this makes
        // it impossible for one to bleed into the next pane if it ever does not.
        self.push_clip(v.layout.breadcrumb);
        let crumbs = v.layout.crumbs.clone();
        let last = crumbs.len().saturating_sub(1);
        for (i, c) in crumbs.iter().enumerate() {
            let hovered = v.hover == Some(Hit::Crumb(v.pid, c.segment_index));
            if hovered {
                self.fill_rounded(c.rect.inset(0, 4), m.radius, p.row_hover);
            }
            let label = if c.is_ellipsis {
                "\u{2026}".to_string()
            } else {
                v.crumbs.get(c.segment_index).cloned().unwrap_or_default()
            };
            let is_leaf = i == last && !c.is_ellipsis;
            let fg = if is_leaf || hovered { p.text } else { p.text_muted };
            let fmt = if is_leaf {
                self.formats.small_bold.clone()
            } else {
                self.formats.small.clone()
            };
            let inner = Rect::new(c.rect.x + m.pad / 2, c.rect.y, c.rect.w - m.pad / 2, c.rect.h);
            self.text(&label, inner, fg, &fmt);

            if i < last {
                let gap = Rect::new(
                    c.rect.right(),
                    c.rect.y,
                    (crumbs[i + 1].rect.x - c.rect.right()).max(0),
                    c.rect.h,
                );
                self.glyph(glyph::CHEVRON_RIGHT, gap, p.text_faint);
            }
        }
        self.pop_clip();
    }

    fn draw_header(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.header, p.header_bg);

        for c in v.layout.columns.clone() {
            let hovered = v.hover == Some(Hit::Column(v.pid, c.key));
            if hovered {
                self.fill(c.rect, p.row_hover);
            }
            let sorted = v.list.sort_key == c.key;
            let title = match c.key {
                SortKey::Name => "Name",
                SortKey::Type => "Type",
                SortKey::Size => "Size",
                SortKey::Date => "Date modified",
            };
            let right_aligned = matches!(c.key, SortKey::Size);
            let arrow_w = if sorted { m.pad * 2 } else { 0 };

            let fmt = if right_aligned {
                self.formats.small_right.clone()
            } else if sorted {
                self.formats.small_bold.clone()
            } else {
                self.formats.small.clone()
            };
            let inner = if right_aligned {
                Rect::new(
                    c.rect.x,
                    c.rect.y,
                    (c.rect.w - m.pad - arrow_w).max(0),
                    c.rect.h,
                )
            } else {
                Rect::new(
                    c.rect.x + m.pad,
                    c.rect.y,
                    (c.rect.w - m.pad * 2 - arrow_w).max(0),
                    c.rect.h,
                )
            };
            let fg = if sorted { p.text } else { p.text_muted };
            self.text(title, inner, fg, &fmt);

            if sorted {
                let g = if v.list.sort_order == SortOrder::Asc {
                    glyph::CHEVRON_UP
                } else {
                    glyph::CHEVRON_DOWN
                };
                // Sit the marker against the title, not at the far edge of a
                // wide column where it reads as belonging to nothing.
                let title_w = self.measure(title, true).ceil() as i32;
                let ar = if right_aligned {
                    Rect::new(inner.x - arrow_w, c.rect.y, arrow_w, c.rect.h)
                } else {
                    Rect::new(inner.x + title_w, c.rect.y, arrow_w, c.rect.h)
                };
                let ar = Rect::new(
                    ar.x.min(c.rect.right() - arrow_w),
                    ar.y,
                    arrow_w,
                    ar.h,
                );
                self.glyph(g, ar, p.accent);
            }
        }

        let underline = Rect::new(
            v.layout.header.x,
            v.layout.header.bottom() - 1,
            v.layout.header.w,
            1,
        );
        self.fill(underline, p.divider);
    }

    fn draw_rows(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        let list_rect = v.layout.list;

        if let Some(err) = v.error {
            self.centered_message(list_rect, &format!("Cannot open this folder\n{}", err));
            return;
        }
        if v.loading && v.list.entries.is_empty() {
            self.centered_message(list_rect, "Loading\u{2026}");
            return;
        }
        if v.list.entries.is_empty() {
            let msg = if v.list.is_filtered() {
                "No items match the filter"
            } else {
                "This folder is empty"
            };
            self.centered_message(list_rect, msg);
            return;
        }

        self.push_clip(list_rect);

        let row_h = v.layout.line_h;
        let grid = v.layout.columns_per_line > 1;
        let (start, end) = v.list.visible_range(list_rect.h as u32);
        let cursor = v.list.cursor();

        // Column geometry comes from the same layout the header used, so the
        // cells line up with their titles by construction.
        let cols = v.layout.columns.clone();
        let type_col = cols.iter().find(|c| c.key == SortKey::Type).map(|c| c.rect);
        let size_col = cols.iter().find(|c| c.key == SortKey::Size).map(|c| c.rect);
        let date_col = cols.iter().find(|c| c.key == SortKey::Date).map(|c| c.rect);

        for row in start..end {
            let Some(entry) = v.list.entries.get(row as usize) else {
                break;
            };
            let Some(r) = v.layout.cell(row, v.list.scroll_px()) else {
                continue;
            };
            let y = r.y;

            let selected = v.list.is_selected(row);
            let hovered = v.hover == Some(Hit::Row(v.pid, row));

            if selected {
                let c = if v.focused {
                    p.selection
                } else {
                    p.selection_inactive
                };
                // A rounded pill inset from the gutter reads as a selected
                // object; a full-bleed rectangle reads as a stripe.
                self.fill_rounded(Rect::new(r.x + 2, r.y, r.w - 4, r.h), m.radius, c);
            } else if hovered {
                self.fill_rounded(Rect::new(r.x + 2, r.y, r.w - 4, r.h), m.radius, p.row_hover);
            }
            if Some(row) == cursor && v.focused && !selected {
                self.stroke_rounded(
                    Rect::new(r.x + 2, r.y, r.w - 4, r.h),
                    m.radius,
                    p.cursor_outline,
                    1.0,
                );
            }

            let fg = if selected { p.text_on_selection } else { p.text };
            let muted = if selected {
                p.text_on_selection
            } else {
                p.text_muted
            };

            // Compare mode: an accent bar for what only this pane has, and a
            // warning bar for a file that is here too but holds different
            // bytes. Two colours rather than two columns: the marker is meant
            // to be scanned, not read.
            let missing = v.other_names.is_some_and(|o| !o.contains(&entry.name));
            let differs = v.list.differing.contains(&entry.name);
            if missing || differs {
                let bar = Rect::new(r.x, r.y + 2, 3.max(m.scale as i32), r.h - 4);
                self.fill_rounded(bar, 1.5, if differs { p.capacity_full } else { p.accent });
            }

            if let Some((icon, name)) = v.layout.cell_parts(row, v.list.scroll_px(), m) {
                // The shell's image if a worker has produced one, and the
                // file-type icon until then, so no cell is ever empty.
                let drawn = match v.thumbs {
                    Some((cache, dir)) => {
                        let key = crate::app::AppState::image_key(dir, entry);
                        self.draw_cell_image(&key, cache, icon)
                    }
                    None => false,
                };
                if !drawn {
                    let key = icon_key(entry.extension.as_deref(), entry.is_dir);
                    let small = if grid {
                        // A 16px icon blown up to 72 is a smear. Draw it at its
                        // own size in the middle of the cell instead.
                        Rect::new(
                            icon.x + (icon.w - m.icon_size) / 2,
                            icon.y + (icon.h - m.icon_size) / 2,
                            m.icon_size,
                            m.icon_size,
                        )
                    } else {
                        icon
                    };
                    self.draw_icon(&key, small);
                }

                // The row being renamed shows the edit box instead of its name.
                if v.renaming != Some(row) {
                    let fmt = if grid {
                        // Two centred lines under the icon; a longer name is
                        // trimmed rather than pushed into the next cell.
                        self.formats.cell_label.clone()
                    } else {
                        self.formats.body.clone()
                    };
                    self.text(&entry.name, name, fg, &fmt);
                }

                // Junctions and symlinks are marked, so a recursive copy is
                // never a surprise.
                if entry.is_reparse {
                    let badge = Rect::new(icon.x, icon.bottom() - 5, 5, 5);
                    self.fill_rounded(badge, 2.5, p.accent);
                }
            }

            // The columns are the details view. The icon view has none, and
            // its cell layout already gave the name the whole width.
            if grid {
                continue;
            }

            if let Some(tc) = type_col {
                let cell = Rect::new(tc.x + m.pad, y, tc.w - m.pad * 2, row_h);
                self.text(
                    &entry.type_display(),
                    cell,
                    muted,
                    &self.formats.body.clone(),
                );
            }
            if let Some(sc) = size_col {
                let cell = Rect::new(sc.x, y, sc.w - m.pad, row_h);
                self.text(
                    &entry.size_display(),
                    cell,
                    muted,
                    &self.formats.body_right.clone(),
                );
            }
            if let Some(dc) = date_col {
                let cell = Rect::new(dc.x + m.pad, y, dc.w - m.pad * 2, row_h);
                self.text(&entry.date_display(), cell, muted, &self.formats.body.clone());
            }
        }

        self.pop_clip();
    }

    fn centered_message(&mut self, area: Rect, msg: &str) {
        let p = self.palette();
        let h = 40;
        let r = Rect::new(area.x, area.y + (area.h - h) / 2, area.w, h);
        self.text(msg, r, p.text_faint, &self.formats.centered.clone());
    }

    fn draw_scrollbar(&mut self, v: &PaneView) {
        let p = self.palette();
        if v.layout.scrollbar.is_empty() || v.layout.thumb.is_empty() {
            return;
        }
        let hovered = matches!(v.hover, Some(Hit::ScrollbarThumb(s)) if s == v.pid);
        let c = if hovered {
            p.scrollbar_thumb_hover
        } else {
            p.scrollbar_thumb
        };
        let thumb = v.layout.thumb.inset(3, 2);
        self.fill_rounded(thumb, thumb.w as f32 / 2.0, c);
    }

    fn draw_footer(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.footer, p.footer_bg);
        let top = Rect::new(v.layout.footer.x, v.layout.footer.y, v.layout.footer.w, 1);
        self.fill(top, p.divider);

        // Filter field.
        let f = v.layout.filter;
        if !f.is_empty() {
            let hovered = v.hover == Some(Hit::Filter(v.pid));
            self.fill_rounded(f, m.radius, p.field_bg);
            if v.filter_focused {
                self.stroke_rounded(f, m.radius, p.accent, 1.0);
            } else if hovered {
                self.stroke_rounded(f, m.radius, p.divider, 1.0);
            }

            let gi = Rect::new(f.x + 2, f.y, m.icon_size, f.h);
            self.glyph(glyph::FILTER, gi, p.text_faint);

            let text_rect = Rect::new(gi.right(), f.y, (f.right() - gi.right() - m.pad / 2).max(0), f.h);
            let filter = v.list.filter();
            if filter.is_empty() {
                self.text("Filter\u{2026}", text_rect, p.text_faint, &self.formats.tiny.clone());
            } else {
                let shown = if v.filter_focused {
                    format!("{}|", filter)
                } else {
                    filter.to_string()
                };
                self.text(&shown, text_rect, p.text, &self.formats.tiny.clone());
            }
        }

        let counts = Rect::new(
            v.layout.counts.x,
            v.layout.counts.y,
            (v.layout.counts.w - m.pad).max(0),
            v.layout.counts.h,
        );
        self.text(v.counts, counts, p.text_muted, &self.formats.small_right.clone());
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
