// The context menu, drawn in the app's own palette.
//
// A Win32 menu follows the *system* theme, so a dark app on a light Windows
// gets a light menu hanging off it, and vice versa. Our own items are
// MF_OWNERDRAW and painted here.
//
// The shell's items are deliberately left alone: they belong to the handlers
// that added them, some of them owner-draw their own icons, and repainting
// another component's menu entries is how you lose the icons. Only the
// background is set for the whole popup, via MIM_BACKGROUND, so the two halves
// at least sit on the same colour.

use std::cell::RefCell;
use std::collections::HashSet;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, ODT_MENU};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::dialog::UiFont;
use crate::theme::{Palette, Rgb, Theme};

fn colorref(c: Rgb) -> COLORREF {
    let q = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    COLORREF(q(c.0) | (q(c.1) << 8) | (q(c.2) << 16))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// One owner-drawn entry. Boxed and handed to Win32 as `dwItemData`, so it has
/// to outlive `TrackPopupMenu` — `ThemedMenu` owns them all until it drops.
struct MenuItem {
    text: Vec<u16>,
    accel: Vec<u16>,
    separator: bool,
}

struct Ctx {
    font: HFONT,
    palette: Palette,
    row_h: i32,
    pad: i32,
    /// Exactly which `dwItemData` values are ours. A pointer check rather than
    /// an id range, because the shell's items can be owner-drawn too and a
    /// wrong guess means drawing over somebody else's entry.
    ours: HashSet<usize>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Owns the owner-draw items and the menu's background brush for as long as
/// the menu is on screen.
pub struct ThemedMenu {
    items: Vec<Box<MenuItem>>,
    back: HBRUSH,
    // Kept alive for the menu's lifetime; the HFONT in CTX points at it.
    _font: Option<UiFont>,
}

impl ThemedMenu {
    pub fn new(theme: Theme, dpi: u32) -> ThemedMenu {
        let palette = theme.palette();
        let font = UiFont::new(dpi);
        let scale = dpi as f32 / 96.0;
        let back = unsafe { CreateSolidBrush(colorref(palette.pane_bg)) };
        CTX.with(|c| {
            *c.borrow_mut() = Some(Ctx {
                font: font.as_ref().map(|f| f.handle()).unwrap_or_default(),
                palette: *palette,
                row_h: (26.0 * scale).round() as i32,
                pad: (10.0 * scale).round() as i32,
                ours: HashSet::new(),
            });
        });
        ThemedMenu {
            items: Vec::new(),
            back,
            _font: font,
        }
    }

    fn push(&mut self, menu: HMENU, id: usize, item: MenuItem) {
        let boxed = Box::new(item);
        let ptr = &*boxed as *const MenuItem as usize;
        self.items.push(boxed);
        CTX.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.ours.insert(ptr);
            }
        });
        unsafe {
            // With MF_OWNERDRAW the last argument is not a string: it is the
            // `dwItemData` the draw and measure messages come back with.
            let _ = AppendMenuW(menu, MF_OWNERDRAW, id, PCWSTR(ptr as *const u16));
        }
    }

    pub fn add(&mut self, menu: HMENU, id: usize, text: &str, accel: &str) {
        self.push(
            menu,
            id,
            MenuItem {
                text: wide(text),
                accel: wide(accel),
                separator: false,
            },
        );
    }

    /// Id 0, which `TrackPopupMenu` reports the same as "nothing was chosen".
    pub fn separator(&mut self, menu: HMENU) {
        self.push(
            menu,
            0,
            MenuItem {
                text: Vec::new(),
                accel: Vec::new(),
                separator: true,
            },
        );
    }

    /// Paint the popup's own background, including the strip beside the shell's
    /// items, so the two halves sit on one colour.
    pub fn apply(&self, menu: HMENU) {
        let info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_BACKGROUND | MIM_APPLYTOSUBMENUS,
            hbrBack: self.back,
            ..Default::default()
        };
        unsafe {
            let _ = SetMenuInfo(menu, &info);
        }
    }
}

impl Drop for ThemedMenu {
    fn drop(&mut self) {
        CTX.with(|c| *c.borrow_mut() = None);
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.back.0));
        }
    }
}

/// Handle WM_MEASUREITEM / WM_DRAWITEM for our own entries.
///
/// Returns None for anything that is not ours, so the caller can pass it on to
/// the shell's context-menu handler.
pub fn handle_menu_msg(msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_MEASUREITEM => unsafe {
            let mis = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
            if mis.CtlType != ODT_MENU || !is_ours(mis.itemData) {
                return None;
            }
            let item = &*(mis.itemData as *const MenuItem);
            CTX.with(|c| {
                let b = c.borrow();
                let ctx = b.as_ref()?;
                if item.separator {
                    mis.itemWidth = 0;
                    mis.itemHeight = (ctx.pad).max(5) as u32;
                    return Some(LRESULT(1));
                }
                let text = measure(ctx.font, &item.text);
                let accel = measure(ctx.font, &item.accel);
                // Four pads: leading, between text and accelerator, trailing,
                // and one more so the gap between the two columns reads.
                mis.itemWidth = (text + accel + ctx.pad * 4) as u32;
                mis.itemHeight = ctx.row_h as u32;
                Some(LRESULT(1))
            })
        },

        WM_DRAWITEM => unsafe {
            let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
            if dis.CtlType != ODT_MENU || !is_ours(dis.itemData) {
                return None;
            }
            draw(dis);
            Some(LRESULT(1))
        },

        _ => None,
    }
}

fn is_ours(data: usize) -> bool {
    data != 0 && CTX.with(|c| c.borrow().as_ref().is_some_and(|x| x.ours.contains(&data)))
}

unsafe fn measure(font: HFONT, text: &[u16]) -> i32 {
    if text.len() <= 1 {
        return 0;
    }
    let hdc = GetDC(None);
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    let mut size = Default::default();
    // Without the trailing NUL: GetTextExtentPoint32W measures what it is given.
    let _ = GetTextExtentPoint32W(hdc, &text[..text.len() - 1], &mut size);
    SelectObject(hdc, old);
    ReleaseDC(None, hdc);
    size.cx
}

unsafe fn draw(dis: &DRAWITEMSTRUCT) {
    let item = &*(dis.itemData as *const MenuItem);
    CTX.with(|c| {
        let b = c.borrow();
        let Some(ctx) = b.as_ref() else { return };
        let p = &ctx.palette;
        let hdc = dis.hDC;
        let r = dis.rcItem;

        fill(hdc, r, p.pane_bg);

        if item.separator {
            let y = (r.top + r.bottom) / 2;
            fill(
                hdc,
                RECT {
                    left: r.left + ctx.pad,
                    top: y,
                    right: r.right - ctx.pad,
                    bottom: y + 1,
                },
                p.divider,
            );
            return;
        }

        let selected = dis.itemState.0 & ODS_SELECTED.0 != 0;
        let disabled = dis.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0;
        if selected && !disabled {
            // Inset and rounded, the way Windows 11 highlights a menu row. A
            // full-bleed rectangle is what makes a menu look like Windows 95.
            let pad = 3;
            let brush = CreateSolidBrush(colorref(p.selection));
            let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let _ = RoundRect(
                hdc,
                r.left + pad,
                r.top,
                r.right - pad,
                r.bottom,
                8,
                8,
            );
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }

        SetBkMode(hdc, TRANSPARENT);
        let old_font = SelectObject(hdc, HGDIOBJ(ctx.font.0));

        let fg = if disabled {
            p.text_faint
        } else if selected {
            p.text_on_selection
        } else {
            p.text
        };
        let mut text_rect = RECT {
            left: r.left + ctx.pad * 2,
            top: r.top,
            right: r.right - ctx.pad,
            bottom: r.bottom,
        };
        SetTextColor(hdc, colorref(fg));
        DrawTextW(
            hdc,
            &mut item.text.clone()[..item.text.len() - 1],
            &mut text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX,
        );

        if item.accel.len() > 1 {
            SetTextColor(hdc, colorref(if selected { p.text_on_selection } else { p.text_faint }));
            let mut accel_rect = RECT {
                left: r.left,
                top: r.top,
                right: r.right - ctx.pad * 2,
                bottom: r.bottom,
            };
            DrawTextW(
                hdc,
                &mut item.accel.clone()[..item.accel.len() - 1],
                &mut accel_rect,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
            );
        }

        SelectObject(hdc, old_font);
    });
}

unsafe fn fill(hdc: HDC, r: RECT, c: Rgb) {
    let brush = CreateSolidBrush(colorref(c));
    FillRect(hdc, &r, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_convert_to_the_bgr_order_gdi_wants() {
        // COLORREF is 0x00BBGGRR, which is the opposite of every other API
        // here; getting it backwards turns the accent blue into orange.
        assert_eq!(colorref((1.0, 0.0, 0.0)).0, 0x0000FF);
        assert_eq!(colorref((0.0, 1.0, 0.0)).0, 0x00FF00);
        assert_eq!(colorref((0.0, 0.0, 1.0)).0, 0xFF0000);
    }

    #[test]
    fn colour_channels_are_clamped_not_wrapped() {
        assert_eq!(colorref((2.0, -1.0, 0.5)).0, 0x008000FF);
    }
}
