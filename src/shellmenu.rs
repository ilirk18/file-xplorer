// The real Windows context menu, appended below our own items.
//
// Our menu keeps the things this app does better (copy to other pane, batch
// rename); the shell contributes everything installed software registered —
// 7-Zip, Git, "Open with", TortoiseSVN, Send to. Reimplementing that list is
// not possible, so we ask for it.
//
// Shell commands are allocated ids from SHELL_ID_FIRST upward, well clear of
// our own, so one TrackPopupMenu can return either kind and the caller can tell
// them apart.
//
// Some handlers populate their submenus lazily and draw their own items, which
// only works if WM_INITMENUPOPUP / WM_MEASUREITEM / WM_DRAWITEM reach them
// while the menu is up. `handle_menu_msg` is that forwarding, and the window
// procedure must call it.

use std::cell::RefCell;

use crate::pidl::Pidl;
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Shell command ids start here. Our own ids are all below 1000.
pub const SHELL_ID_FIRST: u32 = 0x1000;
const SHELL_ID_LAST: u32 = 0x7000;

thread_local! {
    /// The menu currently on screen, if it wants menu messages.
    static ACTIVE: RefCell<Option<IContextMenu2>> = const { RefCell::new(None) };
}

/// A live shell context menu. Holding it keeps the COM object alive for as long
/// as the menu is on screen and a command might still be invoked.
pub struct ShellMenu {
    menu: IContextMenu,
}

/// Append the shell's items for `items` to `hmenu`.
///
/// Takes id lists rather than paths: a namespace item's parsing name is not the
/// item — a Recycle Bin entry's is the path it came from, and binding to that
/// string produces the original file's verbs instead of Restore.
///
/// Returns None when there is nothing to add, which includes the ordinary case
/// of an empty selection — the caller just shows its own menu.
pub fn append(hmenu: HMENU, hwnd: HWND, items: &[Pidl]) -> Option<ShellMenu> {
    if items.is_empty() {
        return None;
    }
    unsafe {
        // Every selected item shares a parent folder, which is what lets one
        // GetUIObjectOf cover the whole selection.
        let parent: IShellFolder = SHBindToParent(items[0].as_ptr(), None).ok()?;
        let children: Vec<_> = items.iter().map(|p| p.child_ptr()).collect();
        let menu: IContextMenu = parent.GetUIObjectOf(hwnd, &children, None).ok()?;

        let count = GetMenuItemCount(Some(hmenu)).max(0) as u32;
        let hr = menu.QueryContextMenu(
            hmenu,
            count,
            SHELL_ID_FIRST,
            SHELL_ID_LAST,
            CMF_NORMAL | CMF_EXPLORE,
        );
        if hr.is_err() {
            return None;
        }

        // Remember the menu for the duration of tracking so lazily-built
        // submenus can populate themselves.
        let forwarding: Option<IContextMenu2> = menu.cast().ok();
        ACTIVE.with(|a| *a.borrow_mut() = forwarding);

        Some(ShellMenu { menu })
    }
}

impl ShellMenu {
    /// Run a shell command id returned by TrackPopupMenu.
    pub fn invoke(&self, hwnd: HWND, cmd: u32) {
        if !(SHELL_ID_FIRST..=SHELL_ID_LAST).contains(&cmd) {
            return;
        }
        // The verb is the offset from idCmdFirst, passed in the low word of a
        // pointer-sized field. This is the documented MAKEINTRESOURCE idiom.
        let offset = (cmd - SHELL_ID_FIRST) as usize;
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR(offset as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe {
            let _ = self.menu.InvokeCommand(&info);
        }
    }
}

impl Drop for ShellMenu {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = None);
    }
}

/// Forward the messages a shell menu needs while it is on screen.
/// Returns Some when the message was consumed.
pub fn handle_menu_msg(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        return None;
    }
    ACTIVE.with(|a| {
        let borrow = a.borrow();
        let menu = borrow.as_ref()?;
        // IContextMenu3 knows how to return a result (needed for WM_MENUCHAR);
        // IContextMenu2 is the older interface that cannot.
        if let Ok(m3) = menu.cast::<IContextMenu3>() {
            let mut result = LRESULT(0);
            unsafe {
                m3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)).ok()?;
            }
            return Some(result);
        }
        unsafe {
            menu.HandleMenuMsg(msg, wparam, lparam).ok()?;
        }
        Some(LRESULT(0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_ids_do_not_collide_with_app_commands() {
        // Application command ids live in the low hundreds; the shell range
        // must start well above them or TrackPopupMenu results are ambiguous.
        assert!(SHELL_ID_FIRST > 1000);
        assert!(SHELL_ID_LAST > SHELL_ID_FIRST);
    }

    #[test]
    fn empty_selection_has_no_shell_menu() {
        // Cheap guard, but it is the path taken on every right-click in empty
        // space, and SHParseDisplayName on nothing is not free.
        assert!(append(HMENU::default(), HWND::default(), &[]).is_none());
    }
}
