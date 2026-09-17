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
/// What the UI is drawn in unless the settings say otherwise.
pub const DEFAULT_FONT: &str = "Segoe UI";

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

    // The window buttons. Segoe MDL2 draws these at the weight Windows' own
    // caption uses, so they sit at the top right looking like they belong.
    pub const MINIMIZE: &str = "\u{E921}";
    pub const MAXIMIZE: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";

    /// A window with its left column filled: the sidebar, shown or not. One
    /// glyph for both states, because a toggle that swaps its picture makes
    /// you work out which picture means which; this one says it in colour.
    pub const SIDEBAR: &str = "\u{E90D}";
    pub const HOME: &str = "\u{E80F}";
    pub const PIN: &str = "\u{E718}";
    pub const UNPIN: &str = "\u{E77A}";
}

/// The overflow menu's button. Segoe MDL2 has no vertical ellipsis, so this
/// one is a character in the UI font rather than a glyph in the icon font.
pub const MORE: &str = "\u{22EE}";

use crate::fs::wide;

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
    /// Is this pane's folder already pinned? The pin button is a toggle, so
    /// its icon is also how the answer is shown.
    pub pinned: bool,
    /// True when keystrokes are going to the filter box rather than the list.
    pub filter_focused: bool,
    /// The rubber-band rectangle being dragged in this pane, already clipped
    /// to the rows area.
    pub band: Option<Rect>,
    /// The row whose name is being edited in place, if it is in this pane.
    pub renaming: Option<u32>,
    /// Rows being typed over together, if this is the pane holding them.
    pub multi: Option<&'a crate::multi_rename::MultiRename>,
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
    /// False when the panel is describing the folder itself because nothing in
    /// it is selected. No worker was sent for that, so there is nothing to wait
    /// for and nothing to say about waiting.
    pub selected: bool,
}

/// The window's own furniture in the caption strip: everything up there that
/// is not a pane's tab bar.
pub struct CaptionView {
    pub sidebar_visible: bool,
    pub maximized: bool,
    pub hover: Option<Hit>,
    /// Which window button the pointer is over: 0 minimise, 1 maximise,
    /// 2 close. They are hit-tested as non-client area, so they do not come
    /// through `hover` like everything else does.
    pub button_hover: Option<usize>,
}

/// What the close button goes under the pointer. Every other window on the
/// desktop does this, so anything else there reads as a different button.
const CLOSE_HOVER: crate::theme::Rgb = (0.77, 0.17, 0.11);

/// Everything the renderer needs to paint the sidebar.
pub struct SidebarView<'a> {
    pub entries: &'a [SidebarEntry],
    pub drives: &'a [Drive],
    /// (label, path) shortcuts.
    /// (label, path, icon cache key) per row of the Places section.
    pub places: &'a [(String, String, String)],
    pub current_path: &'a str,
    /// Visible rows of the folder tree, indexed by `SidebarEntry::Tree`.
    pub tree: &'a [crate::tree::TreeRow],
    pub hover: Option<Hit>,
}

struct Formats {
    /// The inspector's heading: the one place a name is a title rather than a
    /// row.
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    body_right: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_bold: IDWriteTextFormat,
    small_right: IDWriteTextFormat,
    tiny: IDWriteTextFormat,
    tiny_right: IDWriteTextFormat,
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
    /// The UI font family and its size as a percentage. Kept so the formats
    /// can be rebuilt at a new DPI without asking the caller again.
    font: (String, i32),

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
    pub fn new(theme: Theme, dpi: u32, font: &str, font_pct: i32) -> Result<Self> {
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let formats = Self::make_formats(&dwrite, dpi, font, font_pct)?;
        Ok(Self {
            factory,
            dwrite,
            wic,
            target: None,
            brush: None,
            formats,
            theme,
            dpi,
            font: (font.to_string(), font_pct),
            icons: IconCache::default(),
            icon_bitmaps: HashMap::new(),
            preview_bitmap: None,
            cell_bitmaps: HashMap::new(),
            measure_cache: RefCell::new(HashMap::new()),
        })
    }

    fn make_formats(
        dwrite: &IDWriteFactory,
        dpi: u32,
        family: &str,
        font_pct: i32,
    ) -> Result<Formats> {
        // A family DirectWrite does not know falls back to a default at draw
        // time rather than failing here, which is the behaviour we want: a
        // typo in the settings file should not be a window that will not open.
        let scale = dpi as f32 / 96.0
            * (font_pct.clamp(crate::layout::FONT_MIN, crate::layout::FONT_MAX) as f32)
            / 100.0;
        let ui = wide(if family.trim().is_empty() {
            DEFAULT_FONT
        } else {
            family.trim()
        });
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
            title: mk(&ui, 15.0, SEMI, LEADING, true)?,
            body: mk(&ui, 12.5, NORMAL, LEADING, true)?,
            body_right: mk(&ui, 12.5, NORMAL, TRAILING, true)?,
            small: mk(&ui, 12.0, NORMAL, LEADING, true)?,
            small_bold: mk(&ui, 12.0, SEMI, LEADING, true)?,
            small_right: mk(&ui, 12.0, NORMAL, TRAILING, true)?,
            tiny: mk(&ui, 10.5, NORMAL, LEADING, true)?,
            tiny_right: mk(&ui, 10.5, NORMAL, TRAILING, true)?,
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

    /// Change the UI font. The glyph font is not a choice: it is the icon set.
    pub fn set_font(&mut self, family: &str, font_pct: i32) -> Result<()> {
        self.font = (family.to_string(), font_pct);
        self.formats = Self::make_formats(&self.dwrite, self.dpi, family, font_pct)?;
        self.measure_cache.borrow_mut().clear();
        Ok(())
    }

    /// Every font family installed, sorted. Asked once, when the picker opens.
    pub fn font_families(&self) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let mut collection = None;
            if self
                .dwrite
                .GetSystemFontCollection(&mut collection, false)
                .is_err()
            {
                return out;
            }
            let Some(collection) = collection else {
                return out;
            };
            for i in 0..collection.GetFontFamilyCount() {
                let Ok(family) = collection.GetFontFamily(i) else {
                    continue;
                };
                let Ok(names) = family.GetFamilyNames() else {
                    continue;
                };
                // Index 0 is the family's own preferred name; asking for the
                // user's locale and falling back to it is more code than this
                // list is worth.
                let Ok(len) = names.GetStringLength(0) else {
                    continue;
                };
                let mut buf = vec![0u16; len as usize + 1];
                if names.GetString(0, &mut buf).is_err() {
                    continue;
                }
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                out.push(String::from_utf16_lossy(&buf[..end]));
            }
        }
        out.sort();
        out
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
        self.formats = Self::make_formats(&self.dwrite, dpi, &self.font.0, self.font.1)?;
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

    /// The app's mark: an opening, and the jamb beside it running past the
    /// opening top and bottom.
    ///
    /// Drawn rather than blitted from the icon resource, so it follows the
    /// theme's accent and the monitor's scale. The geometry is the icon's, on
    /// the same 32-unit grid, so the caption and the taskbar agree.
    fn draw_logo(&mut self, slot: Rect, m: Metrics, c: Rgb) {
        let side = m.icon_size + m.pad / 2;
        if slot.w < side || slot.h < side {
            return;
        }
        let k = side as f32 / 32.0;
        let at = |x: f32, y: f32, w: f32, h: f32| {
            Rect::new(
                slot.x + m.pad / 2 + (x * k) as i32,
                slot.y + (slot.h - side) / 2 + (y * k) as i32,
                (w * k).round() as i32,
                (h * k).round() as i32,
            )
        };
        self.stroke_rounded(at(3.0, 7.0, 18.0, 18.0), 3.0 * k, c, (3.0 * k).max(1.0));
        self.fill_rounded(at(24.0, 2.0, 5.0, 28.0), 2.5 * k, c);
    }

    /// The strip across the top of the window. The panes have already painted
    /// their tab bars into it by the time this runs, so all that is left is
    /// the two ends: the app icon and the sidebar toggle on the left, the
    /// window buttons on the right.
    pub fn draw_caption(&mut self, layout: &Layout, v: &CaptionView) {
        let cap = layout.caption;
        if cap.is_empty() {
            return;
        }
        let p = self.palette();
        let m = layout.metrics;
        // The sidebar and the inspector stop below the caption, so the strip
        // above each of them is the one part no pane covered.
        for r in [layout.sidebar, layout.inspector] {
            if !r.is_empty() {
                self.fill(Rect::new(r.x, cap.y, r.w, cap.h), p.tab_bar_bg);
            }
        }

        self.draw_logo(layout.caption_logo, m, p.accent);

        let toggle = layout.caption_sidebar;
        if !toggle.is_empty() {
            let hovered = v.hover == Some(Hit::SidebarToggle);
            if hovered {
                self.fill_rounded(toggle.inset(2, 3), m.radius, p.tab_hover);
            }
            let c = match (v.sidebar_visible, hovered) {
                (true, _) => p.accent,
                (false, true) => p.text,
                (false, false) => p.text_muted,
            };
            self.glyph(glyph::SIDEBAR, toggle, c);
        }

        let buttons = [
            (layout.win_min, glyph::MINIMIZE),
            (
                layout.win_max,
                if v.maximized {
                    glyph::RESTORE
                } else {
                    glyph::MAXIMIZE
                },
            ),
            (layout.win_close, glyph::CLOSE),
        ];
        for (i, (r, g)) in buttons.iter().enumerate() {
            let hovered = v.button_hover == Some(i);
            let close = i == 2;
            if hovered {
                self.fill(*r, if close { CLOSE_HOVER } else { p.row_hover });
            }
            let fg = if hovered && close {
                (1.0, 1.0, 1.0)
            } else if hovered {
                p.text
            } else {
                p.text_muted
            };
            self.glyph(g, *r, fg);
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
        let hair = 1.max(m.scale as i32);

        // The name is a title, not a row: it is the only thing in the panel
        // that says what everything else is about.
        let mut y = inner.y;
        self.text(
            name,
            Rect::new(inner.x, y, inner.w, line * 5 / 4),
            p.text,
            &self.formats.title.clone(),
        );
        y += line * 5 / 4;
        if !v.kind.is_empty() {
            self.text(
                &v.kind,
                Rect::new(inner.x, y, inner.w, line * 3 / 4),
                p.text_muted,
                &self.formats.tiny.clone(),
            );
            y += line * 3 / 4;
        }
        y += m.pad;

        // Facts as a two-column table: the label says what the number is, and
        // the numbers line up with each other down the right-hand edge.
        let label_w = inner.w * 2 / 5;
        for (label, value) in [("Size", &v.size), ("Modified", &v.modified)] {
            if value.is_empty() {
                continue;
            }
            self.fill(Rect::new(inner.x, y, inner.w, hair), p.divider);
            y += m.pad / 2;
            let h = line * 4 / 5;
            self.text(
                label,
                Rect::new(inner.x, y, label_w, h),
                p.text_faint,
                &self.formats.tiny.clone(),
            );
            self.text(
                value,
                Rect::new(inner.x + label_w, y, inner.w - label_w, h),
                p.text,
                &self.formats.small_right.clone(),
            );
            y += h + m.pad / 2;
        }

        let body = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(0));
        if body.h <= line {
            self.pop_clip();
            return;
        }
        match v.preview {
            Some((key, crate::preview::Preview::Image(hbm))) => {
                let body = self.section(body, "PREVIEW", "", m, p);
                self.draw_preview_image(key, *hbm, body)
            }
            Some((_, crate::preview::Preview::Text(text))) => {
                let body = self.section(body, "PREVIEW", "", m, p);
                self.text(text, body, p.text_muted, &self.formats.preview.clone())
            }
            // Peeking into a folder: the same icons the listing draws, and a
            // count that says whether this is all of it.
            Some((_, crate::preview::Preview::Folder { items, more })) => {
                let total = items.len() + more;
                let count = if *more > 0 {
                    format!("{} of {}", items.len(), total)
                } else if total == 1 {
                    "1 item".to_string()
                } else {
                    format!("{} items", total)
                };
                let body = self.section(body, "CONTENTS", &count, m, p);
                self.draw_peek(items, body, m, p);
            }
            Some((_, crate::preview::Preview::None)) => {
                self.text(
                    "No preview for this kind of file.",
                    Rect::new(body.x, body.y + m.pad, body.w, line),
                    p.text_faint,
                    &self.formats.tiny.clone(),
                );
            }
            // Nothing has come back from the worker yet.
            None if v.selected => {
                self.text(
                    "Reading\u{2026}",
                    Rect::new(body.x, body.y + m.pad, body.w, line),
                    p.text_faint,
                    &self.formats.tiny.clone(),
                );
            }
            None => {}
        }
        self.pop_clip();
    }

    /// A rule, a heading and an optional count on its right. Returns what is
    /// left of `area` underneath it.
    fn section(&mut self, area: Rect, label: &str, right: &str, m: Metrics, p: &Palette) -> Rect {
        let hair = 1.max(m.scale as i32);
        let h = m.row_h * 4 / 5;
        self.fill(Rect::new(area.x, area.y, area.w, hair), p.divider);
        let y = area.y + m.pad / 2;
        self.text(
            label,
            Rect::new(area.x, y, area.w, h),
            p.text_faint,
            &self.formats.caption.clone(),
        );
        if !right.is_empty() {
            self.text(
                right,
                Rect::new(area.x, y, area.w, h),
                p.text_faint,
                &self.formats.tiny_right.clone(),
            );
        }
        let used = m.pad / 2 + h + m.pad / 2;
        Rect::new(area.x, area.y + used, area.w, (area.h - used).max(0))
    }

    /// The folder peek, one row per entry, stopping at the bottom of the panel
    /// rather than drawing rows nobody can see.
    fn draw_peek(
        &mut self,
        items: &[crate::preview::PeekItem],
        area: Rect,
        m: Metrics,
        p: &Palette,
    ) {
        let h = m.row_h * 4 / 5;
        let gap = m.pad / 2;
        let mut y = area.y;
        for it in items {
            if y + h > area.bottom() {
                break;
            }
            let icon = Rect::new(
                area.x,
                y + (h - m.icon_size) / 2,
                m.icon_size,
                m.icon_size,
            );
            self.draw_icon(&icon_key(it.extension.as_deref(), it.is_dir), icon);
            let text_x = area.x + m.icon_size + gap;
            self.text(
                &it.name,
                Rect::new(text_x, y, (area.right() - text_x).max(0), h),
                if it.is_dir { p.text } else { p.text_muted },
                &self.formats.tiny.clone(),
            );
            y += h;
        }
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

    /// Where a dragged tab would land: the half of the pane it would take,
    /// washed in the accent colour with a line around it.
    ///
    /// Drawn last, over the panes, because it is about to cover one of them.
    pub fn draw_drop_target(&mut self, r: Rect, m: Metrics) {
        if r.is_empty() {
            return;
        }
        let accent = self.palette().accent;
        self.fill_alpha(r, accent, 0.18);
        let t = 2.max(m.scale as i32 * 2);
        for edge in [
            Rect::new(r.x, r.y, r.w, t),
            Rect::new(r.x, r.bottom() - t, r.w, t),
            Rect::new(r.x, r.y, t, r.h),
            Rect::new(r.right() - t, r.y, t, r.h),
        ] {
            self.fill_alpha(edge, accent, 0.9);
        }
    }

    fn fill_alpha(&mut self, r: Rect, c: Rgb, alpha: f32) {
        let Some(brush) = self.brush.clone() else { return };
        let mut colour = color(c);
        colour.a = alpha;
        unsafe { brush.SetColor(&colour) };
        if let Some(rt) = self.rt() {
            unsafe { rt.FillRectangle(&d2d_rect(r), &brush) };
        }
        // Left as found: every other fill assumes the brush is opaque.
        unsafe { brush.SetOpacity(1.0) };
    }

    /// One name being typed over, with the shared selection and caret.
    ///
    /// The caret is a filled sliver rather than a blinking one: it is drawn in
    /// every edited row at once, and twenty blinking carets is a light show.
    fn draw_edited_name(
        &mut self,
        m: &crate::multi_rename::MultiRename,
        index: usize,
        area: Rect,
        selected: bool,
        p: &Palette,
    ) {
        let text = m.text(index);
        let chars: Vec<char> = text.chars().collect();
        let upto = |n: usize| -> String { chars[..n.min(chars.len())].iter().collect() };
        let fmt = self.formats.body.clone();

        let (a, b) = m.selection_in(index);
        if a < b {
            let x0 = self.measure(&upto(a), false);
            let x1 = self.measure(&upto(b), false);
            let sel = Rect::new(
                area.x + x0 as i32,
                area.y + 2,
                (x1 - x0).max(1.0) as i32,
                (area.h - 4).max(1),
            );
            self.fill(sel, p.selection);
        }
        let fg = if selected { p.text_on_selection } else { p.text };
        self.text(&text, area, fg, &fmt);

        let caret_x = self.measure(&upto(m.caret_in(index)), false);
        let caret = Rect::new(area.x + caret_x as i32, area.y + 2, 2, (area.h - 4).max(1));
        self.fill(caret, p.accent);
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
                    // A rule above every heading but the first, so the list
                    // reads as sections rather than as one long column with
                    // occasional small grey words in it.
                    if i > 0 {
                        self.fill(
                            Rect::new(
                                row.rect.x + m.pad,
                                row.rect.y,
                                (row.rect.w - m.pad * 2).max(0),
                                1.max(m.scale as i32),
                            ),
                            p.divider,
                        );
                    }
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
                    let Some((label, path, key)) = v.places.get(*index) else {
                        continue;
                    };
                    let active = crate::pane::paths_equal(v.current_path, path);
                    let key = key.clone();
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

    /// The rounded plate a sidebar row sits on, and the accent tick that marks
    /// the one you are looking at.
    ///
    /// Shared by a place, a drive and a folder, so all three say "you are here"
    /// the same way instead of three flat greys that only differ by row height.
    fn sidebar_pill(&mut self, m: Metrics, rect: Rect, active: bool, hovered: bool) -> Rect {
        let p = self.palette();
        let pill = Rect::new(
            rect.x + m.pad / 2,
            rect.y + 1,
            (rect.w - m.pad).max(0),
            (rect.h - 2).max(0),
        );
        if active {
            self.fill_rounded(pill, m.radius, p.selection_inactive);
            let tick_h = (pill.h / 2).max(8);
            self.fill_rounded(
                Rect::new(
                    pill.x,
                    pill.y + (pill.h - tick_h) / 2,
                    3.max(m.scale as i32),
                    tick_h,
                ),
                1.5,
                p.accent,
            );
        } else if hovered {
            self.fill_rounded(pill, m.radius, p.row_hover);
        }
        pill
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
        let pill = self.sidebar_pill(m, rect, active, hovered);

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
        let pill = self.sidebar_pill(m, rect, active, hovered);

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
        // The whole top row of the pane, not just the part tabs are allowed
        // in: for a pane on the top row the rest of it is the window's caption,
        // which has to be the same colour or the strip reads as two strips.
        let strip = Rect::new(
            v.layout.bounds.x,
            v.layout.tab_bar.y,
            v.layout.bounds.w,
            v.layout.tab_bar.h,
        );
        self.fill(strip, p.tab_bar_bg);

        for (i, t) in v.layout.tabs.clone().iter().enumerate() {
            // Scrolled out of the strip: no rectangle, nothing to draw.
            if t.full.is_empty() {
                continue;
            }
            let active = i == v.active_tab;
            let hovered = matches!(v.hover, Some(Hit::Tab(s, h)) | Some(Hit::TabClose(s, h)) if s == v.pid && h == i);

            // Flat: the active tab is the pane's own colour continuing up
            // into the strip, so the tab and the listing under it read as one
            // surface. Which pane has focus is said once, by the accent rule
            // under the whole strip, rather than once per tab.
            let card = Rect::new(t.full.x + 1, t.full.y + 2, t.full.w - 2, t.full.h - 2);
            if active {
                self.fill_rounded(card, m.radius, p.tab_active);
                // Square off the bottom so the tab joins the row below it
                // instead of floating above it.
                self.fill(
                    Rect::new(card.x, card.bottom() - m.radius as i32 - 1, card.w, m.radius as i32 + 1),
                    p.tab_active,
                );
            } else if hovered {
                self.fill_rounded(card, m.radius, p.tab_hover);
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
            let next_shown = v.layout.tabs.get(i + 1).is_some_and(|n| !n.full.is_empty());
            if !active && !next_active && next_shown {
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
        let under = Rect::new(strip.x, strip.bottom() - 1, strip.w, 1);
        self.fill(under, p.divider);
        if let Some(t) = v.layout.tabs.get(v.active_tab).filter(|t| !t.full.is_empty()) {
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
            (v.layout.nav_home, NavButton::Home, glyph::HOME, true),
            (
                v.layout.nav_pin,
                NavButton::Pin,
                if v.pinned { glyph::UNPIN } else { glyph::PIN },
                true,
            ),
        ];
        for (rect, which, g, enabled) in buttons {
            let hovered = v.hover == Some(Hit::Nav(v.pid, which));
            if hovered && enabled {
                self.fill_rounded(rect.inset(2, 4), m.radius, p.row_hover);
            }
            // A disabled control that looks enabled is worse than no control.
            let c = if !enabled {
                p.text_faint
            } else if which == NavButton::Pin && v.pinned {
                p.accent
            } else if hovered {
                p.text
            } else {
                p.text_muted
            };
            self.glyph(g, rect, c);
        }

        // Everything the command bar used to hold, for this pane.
        if !v.layout.overflow.is_empty() {
            let hovered = v.hover == Some(Hit::Overflow(v.pid));
            if hovered {
                self.fill_rounded(v.layout.overflow.inset(2, 4), m.radius, p.row_hover);
            }
            let c = if hovered { p.text } else { p.text_muted };
            self.text(MORE, v.layout.overflow, c, &self.formats.centered.clone());
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

            // A row being typed over shows the text selection instead of the
            // row selection: two blues on top of each other is one blue, and
            // the one that matters is the one around the characters.
            let editing = v.multi.is_some_and(|m| m.index_of(row).is_some());
            let selected = v.list.is_selected(row) && !editing;
            let hovered = v.hover == Some(Hit::Row(v.pid, row));

            if editing {
                // Still marked as a row that is in the edit, just quietly.
                self.stroke_rounded(
                    Rect::new(r.x + 2, r.y, r.w - 4, r.h),
                    m.radius,
                    p.accent,
                    1.0,
                );
            }
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
            if Some(row) == cursor && v.focused && !selected && !editing {
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
            // A duplicate listing marks the same edge, in the same shape, but
            // it alternates: what the bar says there is not "this row is
            // special" but "this row and the one above are the same finding".
            // Two neighbouring groups of one colour would read as one group.
            let group = v.list.groups.get(&entry.name).copied();
            if missing || differs || group.is_some() {
                let bar = Rect::new(r.x, r.y + 2, 3.max(m.scale as i32), r.h - 4);
                let colour = match group {
                    Some(g) if g % 2 == 0 => p.accent,
                    Some(_) => p.capacity_fill,
                    None if differs => p.capacity_full,
                    None => p.accent,
                };
                self.fill_rounded(bar, 1.5, colour);
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

                // The row being renamed shows the edit box instead of its
                // name, and one of several being renamed shows what is being
                // typed into all of them.
                if let Some((m, i)) = v.multi.and_then(|m| Some((m, m.index_of(row)?))) {
                    self.draw_edited_name(m, i, name, selected, p);
                } else if v.renaming != Some(row) {
                    let fmt = if grid && !v.layout.side_label {
                        // Two centred lines under the icon; a longer name is
                        // trimmed rather than pushed into the next cell. A
                        // label beside its icon is one line, like a row.
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

    /// The filter box, at the left end of the footer.
    fn draw_filter(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        let f = v.layout.filter;
        if f.is_empty() {
            return;
        }
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

    fn draw_footer(&mut self, m: Metrics, v: &PaneView) {
        let p = self.palette();
        self.fill(v.layout.footer, p.footer_bg);
        let top = Rect::new(v.layout.footer.x, v.layout.footer.y, v.layout.footer.w, 1);
        self.fill(top, p.divider);

        self.draw_filter(m, v);

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
