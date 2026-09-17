// Renaming a file by typing over its name, rather than in a dialog.
//
// The editor is a real Win32 EDIT control parented to the main window and
// positioned at the rectangle `layout::row_cells` gave the name — the same one
// the painter uses, so the text does not shift when the box appears.
//
// A real control rather than a hand-rolled caret because text editing is not
// one feature: it is selection, the clipboard, IME composition, right-to-left,
// undo, and the context menu. All of that arrives for free and all of it would
// otherwise have to be written and got right.
//
// Enter and Escape never reach the window procedure, because the EDIT consumes
// them; `handle_key` is called from the message pump before dispatch and turns
// them into a posted message instead.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VIRTUAL_KEY, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::AppState;
use crate::layout::PaneId;
use crate::dialog::UiFont;

/// Child id of the edit box, so `WM_COMMAND` can tell it apart.
pub const IDC_INLINE_EDIT: i32 = 4001;
/// Posted by `handle_key` and by the kill-focus handler. `wparam` is 1 to
/// commit what was typed, 0 to abandon it.
pub const WM_APP_RENAME_DONE: u32 = WM_APP + 20;

/// EM_SETSEL, as in `prompt.rs`: one constant is not worth another feature.
const EM_SETSEL: u32 = 0x00B1;

use crate::fs::wide;

pub struct InlineRename {
    edit: HWND,
    pub pid: PaneId,
    pub row: u32,
    pub old_name: String,
    /// Owned so it outlives the control, and deleted with it.
    _font: Option<UiFont>,
}

impl InlineRename {
    /// Put an editor over `row` in `pid`, pre-filled with its name and with the
    /// stem selected — the extension is rarely what is being changed.
    ///
    /// `None` when the row is scrolled out of view or has no name cell, which
    /// is the caller's cue to fall back to the modal prompt.
    pub fn begin(parent: HWND, state: &AppState, pid: PaneId, row: u32) -> Option<InlineRename> {
        let name = state
            .pane(pid)
            .list()
            .entries
            .get(row as usize)?
            .name
            .clone();
        let layout = state.layout();
        let scroll = state.pane(pid).list().scroll_px();
        let list = layout.pane(pid).list;
        let (_, cell) = layout.pane(pid).cell_parts(row, scroll, layout.metrics)?;
        // Only a row that is *wholly* on screen can be edited: a box hanging
        // half out of the panel is not usable, and returning None here is the
        // caller's cue to fall back to the dialog.
        if cell.is_empty() || cell.y < list.y || cell.bottom() > list.bottom() {
            return None;
        }

        let class = wide("EDIT");
        let text = wide(&name);
        let edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR::from_raw(class.as_ptr()),
                PCWSTR::from_raw(text.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                cell.x,
                cell.y,
                cell.w.max(60),
                cell.h,
                Some(parent),
                Some(HMENU(IDC_INLINE_EDIT as *mut _)),
                None,
                None,
            )
        }
        .ok()?;

        let font = UiFont::new(state.dpi);
        if let Some(f) = &font {
            unsafe {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(f.handle().0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        unsafe {
            // Select the stem, not the extension: ".txt" is almost never the
            // part being retyped, and a full selection would eat it.
            let stem = crate::batch_rename::split_name(&name).0.chars().count();
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem as isize)));
            let _ = SetFocus(Some(edit));
        }

        Some(InlineRename {
            edit,
            pid,
            row,
            old_name: name,
            _font: font,
        })
    }

    /// What is in the box now.
    pub fn text(&self) -> String {
        unsafe {
            let len = GetWindowTextLengthW(self.edit);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(self.edit, &mut buf);
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

}

impl Drop for InlineRename {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.edit);
        }
    }
}

/// Intercept Enter and Escape on their way to the edit control.
///
/// Called from the message pump before `TranslateMessage`, because an EDIT
/// handles both keys itself and the window procedure never sees them. Returns
/// true when the message has been dealt with and must not be dispatched.
pub fn handle_key(main: HWND, state: &AppState, msg: &MSG) -> bool {
    let Some(r) = &state.rename else {
        return false;
    };
    if msg.message != WM_KEYDOWN || msg.hwnd != r.edit {
        return false;
    }
    let commit = match VIRTUAL_KEY(msg.wParam.0 as u16) {
        VK_RETURN => 1usize,
        VK_ESCAPE => 0,
        _ => return false,
    };
    unsafe {
        let _ = PostMessageW(Some(main), WM_APP_RENAME_DONE, WPARAM(commit), LPARAM(0));
    }
    true
}
