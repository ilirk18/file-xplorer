// The context menu, drawn in the app's own palette.
//
// A Win32 menu follows the *system* theme, so a dark app on a light Windows
// gets a light menu hanging off it, and vice versa. Our own items are
// MF_OWNERDRAW and painted here.
//
// The shell's items are adopted: every entry that is plain text becomes one of
// ours, so the whole menu is one colour. Half a menu in the system's palette is
// the black-text-on-dark that a background brush alone cannot fix. The price is
// the small bitmap Windows drew beside a handler's entry, which is a price a
// menu that reads gets to charge. Entries another component *owner-draws* are
// still left alone: those paint themselves, and we would paint over them.

use std::cell::RefCell;
use std::collections::HashSet;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, ODT_MENU};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::dialog::UiFont;
use crate::theme::{colorref, Palette, Rgb, Theme};


use crate::fs::wide;

/// One owner-drawn entry. Boxed and handed to Win32 as `dwItemData`, so it has
/// to outlive `TrackPopupMenu` — `ThemedMenu` owns them all until it drops.
struct MenuItem {
    text: Vec<u16>,
    accel: Vec<u16>,
    separator: bool,
    /// Opens a submenu. Windows draws the arrow for these even though they are
    /// owner-drawn, so all this does is reserve the room for it.
    popup: bool,
}

struct Ctx {
    font: HFONT,
    palette: Palette,
    row_h: i32,
    pad: i32,
    /// Entries taken over from a menu somebody else filled. Owned here rather
    /// than by `ThemedMenu` because the shell fills a submenu when it opens,
    /// long after the menu was built, and the handler that sees that has only
    /// this.
    adopted: Vec<Box<MenuItem>>,
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
                adopted: Vec::new(),
            });
        });
        ThemedMenu {
            items: Vec::new(),
            back,
            _font: font,
        }
    }

    fn push(&mut self, menu: HMENU, id: usize, item: MenuItem) {
        self.place(menu, None, id, item)
    }

    /// `at` None appends; Some(pos) inserts before that position.
    fn place(&mut self, menu: HMENU, at: Option<u32>, id: usize, item: MenuItem) {
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
            match at {
                None => {
                    let _ = AppendMenuW(menu, MF_OWNERDRAW, id, PCWSTR(ptr as *const u16));
                }
                Some(pos) => {
                    let _ = InsertMenuW(
                        menu,
                        pos,
                        MF_OWNERDRAW | MF_BYPOSITION,
                        id,
                        PCWSTR(ptr as *const u16),
                    );
                }
            }
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
                popup: false,
            },
        );
    }


    /// Put an entry at a position rather than at the end. Used to hoist
    /// pinned shell verbs above everything else, which is the whole point of
    /// pinning one.
    pub fn insert(&mut self, menu: HMENU, at: u32, id: usize, text: &str) {
        self.place(
            menu,
            Some(at),
            id,
            MenuItem {
                text: wide(text),
                accel: Vec::new(),
                separator: false,
                popup: false,
            },
        );
    }

    pub fn insert_separator(&mut self, menu: HMENU, at: u32) {
        self.place(
            menu,
            Some(at),
            0,
            MenuItem {
                text: Vec::new(),
                accel: Vec::new(),
                separator: true,
                popup: false,
            },
        );
    }

    /// Add a submenu and return it, so the caller can fill it.
    ///
    /// With MF_POPUP the id slot carries the child's handle instead of a
    /// command id, which is why this cannot go through `add`. Destroying the
    /// parent destroys the child with it, so ownership does not change.
    ///
    /// Returns None only if Windows refuses to make a menu, in which case the
    /// caller has nothing to fill and the entry is simply not there.
    pub fn submenu(&mut self, menu: HMENU, text: &str) -> Option<HMENU> {
        let child = unsafe { CreatePopupMenu().ok()? };
        // The child inherits the parent's background, and its items are drawn
        // by the same owner-draw path: the pointer check in `ours` is by
        // item, not by menu.
        let info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_BACKGROUND | MIM_APPLYTOSUBMENUS,
            hbrBack: self.back,
            ..Default::default()
        };
        unsafe {
            let _ = SetMenuInfo(child, &info);
        }
        self.place_popup(menu, child, text);
        Some(child)
    }

    fn place_popup(&mut self, menu: HMENU, child: HMENU, text: &str) {
        let boxed = Box::new(MenuItem {
            text: wide(text),
            accel: Vec::new(),
            separator: false,
            popup: true,
        });
        let ptr = &*boxed as *const MenuItem as usize;
        self.items.push(boxed);
        CTX.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.ours.insert(ptr);
            }
        });
        unsafe {
            let _ = AppendMenuW(
                menu,
                MF_OWNERDRAW | MF_POPUP,
                child.0 as usize,
                PCWSTR(ptr as *const u16),
            );
        }
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
                popup: false,
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

/// Nested deeper than any shell menu goes, and a stop if one ever loops.
const MAX_DEPTH: u32 = 3;

/// Draw a menu somebody else filled in our palette too.
///
/// Called once when the shell's items are appended, and again for each submenu
/// as it opens: a handler fills "Send to" only when it is about to be shown,
/// so anything adopted before that would have been an empty menu.
pub fn adopt(menu: HMENU) {
    unsafe { adopt_inner(menu, 0) }
}

unsafe fn adopt_inner(menu: HMENU, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    for i in 0..GetMenuItemCount(Some(menu)).max(0) as u32 {
        let sub = GetSubMenu(menu, i as i32);
        if !sub.is_invalid() {
            adopt_inner(sub, depth + 1);
        }
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE,
            ..Default::default()
        };
        if GetMenuItemInfoW(menu, i, true, &mut info).is_err() {
            continue;
        }
        // Somebody else's owner-drawn entry, or one of ours already.
        if info.fType.0 & MFT_OWNERDRAW.0 != 0 {
            continue;
        }
        let separator = info.fType.0 & MFT_SEPARATOR.0 != 0;
        let mut buf = [0u16; 256];
        let n = GetMenuStringW(menu, i, Some(&mut buf), MF_BYPOSITION);
        if !separator && n <= 0 {
            continue;
        }
        let (text, accel) = split_label(&String::from_utf16_lossy(&buf[..n.max(0) as usize]));
        let item = Box::new(MenuItem {
            text: if separator { Vec::new() } else { wide(&text) },
            accel: if separator { Vec::new() } else { wide(&accel) },
            separator,
            popup: !sub.is_invalid(),
        });
        let ptr = &*item as *const MenuItem as usize;
        let taken = CTX.with(|c| match c.borrow_mut().as_mut() {
            Some(ctx) => {
                ctx.ours.insert(ptr);
                ctx.adopted.push(item);
                true
            }
            // No themed menu is up, so there is nothing to match colours with.
            None => false,
        });
        if !taken {
            return;
        }
        let mut set = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA,
            fType: MENU_ITEM_TYPE(info.fType.0 | MFT_OWNERDRAW.0),
            dwItemData: ptr,
            ..Default::default()
        };
        let _ = SetMenuItemInfoW(menu, i, true, &mut set);
    }
}

/// A menu string is "&Label\tCtrl+X". The ampersand marks the mnemonic and
/// the tab starts the shortcut: both are drawn, neither is text.
fn split_label(raw: &str) -> (String, String) {
    let mut parts = raw.split(['\t', '\u{8}']);
    let label = parts.next().unwrap_or("");
    let accel = parts.next().unwrap_or("").trim().to_string();
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            // "&&" is a literal ampersand; a lone one marks the next letter.
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    (out.trim().to_string(), accel)
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
                // A submenu needs a column for the arrow Windows draws itself:
                // without it the arrow lands on the last letter of the label.
                let arrow = if item.popup { ctx.row_h } else { 0 };
                mis.itemWidth = (text + accel + arrow + ctx.pad * 4) as u32;
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
