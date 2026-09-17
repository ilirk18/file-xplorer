// Shared plumbing for the modal dialogs (prompt, palette, batch rename).
//
// Win32 child controls default to the 1990s bitmap system font unless you tell
// them otherwise, which is why an otherwise fine dialog looks broken. This asks
// the system for the *user's* UI font at the *window's* DPI and applies it to
// every child in one call.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT, HGDIOBJ};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, SendMessageW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, WM_SETFONT,
};

/// A font that deletes itself. Must outlive the dialog that uses it.
pub struct UiFont(HFONT);

impl UiFont {
    /// The user's message-box font, sized for `dpi`.
    pub fn new(dpi: u32) -> Option<UiFont> {
        let mut ncm = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                ncm.cbSize,
                Some(&mut ncm as *mut _ as *mut _),
                0,
                dpi,
            )
            .ok()?;
            let font = CreateFontIndirectW(&ncm.lfMessageFont);
            if font.is_invalid() {
                None
            } else {
                Some(UiFont(font))
            }
        }
    }

    pub fn handle(&self) -> HFONT {
        self.0
    }

    /// Apply to every child control of `parent`.
    pub fn apply_to_children(&self, parent: HWND) {
        unsafe {
            let _ = EnumChildWindows(Some(parent), Some(set_font), LPARAM(self.0 .0 as isize));
        }
    }
}

impl Drop for UiFont {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

unsafe extern "system" fn set_font(child: HWND, lparam: LPARAM) -> BOOL {
    SendMessageW(
        child,
        WM_SETFONT,
        Some(WPARAM(lparam.0 as usize)),
        // Redraw: the control is already visible by the time we get here.
        Some(LPARAM(1)),
    );
    TRUE
}
