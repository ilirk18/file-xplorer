// Drag and drop, both directions.
//
// Dropping in: the window registers an IDropTarget. A drop is turned into a
// list of paths and an effect and posted to the UI thread, which decides the
// destination folder from where the cursor landed and runs the usual
// IFileOperation — so a drag lands in the same undo stack as everything else.
//
// Dragging out: the selection becomes a shell IDataObject via
// SHCreateDataObject, so Explorer, editors and archivers all understand it.
//
// The effect is decided by modifier keys alone — Copy by default, Shift for
// Move, Ctrl for Copy. Explorer additionally defaults to Move within one
// volume, but the drop target cannot know the destination folder while the
// cursor is still moving, so following that rule would mean showing a "copy"
// cursor and then performing a move. A cursor that tells the truth is worth
// more than matching Explorer here, and Copy is the reading that cannot
// destroy anything.

use windows::core::{implement, Ref, Result, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, POINTL, WPARAM};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE,
    DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_RBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS,
};
use windows::Win32::UI::Shell::{SHCreateDataObject, HDROP};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::ops;
use crate::pidl::PidlList;

/// What the UI thread receives when something is dropped on the window.
pub struct Dropped {
    pub paths: Vec<String>,
    /// True when the drop should move rather than copy.
    pub move_it: bool,
    /// Screen coordinates, so the caller can work out which pane and row.
    pub screen: (i32, i32),
}

/// Copy unless Shift is held. Ctrl is accepted as an explicit "copy" so the
/// familiar chord does not simply do nothing.
fn effect_for(keys: MODIFIERKEYS_FLAGS) -> DROPEFFECT {
    if keys.0 & MK_CONTROL.0 != 0 {
        DROPEFFECT_COPY
    } else if keys.0 & MK_SHIFT.0 != 0 {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    }
}

/// Pull the dragged file list out of a data object, if it has one.
fn paths_from(data: &IDataObject) -> Vec<String> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else {
            return Vec::new();
        };
        let paths = ops::paths_from_hdrop(HDROP(medium.u.hGlobal.0));
        ReleaseStgMedium(&mut medium);
        paths
    }
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    message: u32,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let accepted = data
            .as_ref()
            .map(|d| !paths_from(d).is_empty())
            .unwrap_or(false);
        unsafe {
            *effect = if accepted {
                effect_for(keys)
            } else {
                DROPEFFECT_NONE
            };
        }
        Ok(())
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        // Whatever DragEnter decided stays decided, except that the user can
        // change their mind about copy vs move mid-drag.
        unsafe {
            if *effect != DROPEFFECT_NONE {
                *effect = effect_for(keys);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let paths = data.as_ref().map(paths_from).unwrap_or_default();
        let chosen = effect_for(keys);
        unsafe {
            *effect = if paths.is_empty() {
                DROPEFFECT_NONE
            } else {
                chosen
            };
        }
        if paths.is_empty() {
            return Ok(());
        }

        // Hand off to the UI thread: it owns the pane state needed to work out
        // where this landed, and no file operation should start on this one.
        let payload = Box::into_raw(Box::new(Dropped {
            paths,
            move_it: chosen == DROPEFFECT_MOVE,
            screen: (pt.x, pt.y),
        }));
        let posted = unsafe {
            PostMessageW(
                Some(self.hwnd),
                self.message,
                WPARAM(0),
                LPARAM(payload as isize),
            )
        };
        if posted.is_err() {
            unsafe { drop(Box::from_raw(payload)) };
        }
        Ok(())
    }
}

/// Register the window as a drop target. The returned value must be kept alive
/// for as long as the window is; dropping it revokes the registration.
pub struct Registration(HWND);

impl Registration {
    pub fn new(hwnd: HWND, message: u32) -> Option<Registration> {
        let target: IDropTarget = Target { hwnd, message }.into();
        unsafe { RegisterDragDrop(hwnd, &target) }.ok()?;
        Some(Registration(hwnd))
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            let _ = RevokeDragDrop(self.0);
        }
    }
}

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: BOOL,
        keys: MODIFIERKEYS_FLAGS,
    ) -> windows::core::HRESULT {
        const DRAGDROP_S_DROP: windows::core::HRESULT = windows::core::HRESULT(0x00040100u32 as i32);
        const DRAGDROP_S_CANCEL: windows::core::HRESULT =
            windows::core::HRESULT(0x00040101u32 as i32);

        if escape_pressed.as_bool() || keys.0 & MK_RBUTTON.0 != 0 {
            return DRAGDROP_S_CANCEL;
        }
        if keys.0 & MK_LBUTTON.0 == 0 {
            return DRAGDROP_S_DROP;
        }
        windows::core::HRESULT(0) // S_OK: keep dragging
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
        // Let OLE draw the standard copy/move cursors.
        windows::core::HRESULT(0x00040102u32 as i32) // DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Begin dragging `paths` out of this window. Blocks until the drag ends.
/// Returns true if the user moved the files (so the source pane should refresh).
pub fn start_drag(paths: &[String]) -> bool {
    let Some(pidls) = PidlList::absolute(paths) else {
        return false;
    };
    let items = pidls.absolute_ptrs();

    // A NULL parent folder means the item ids are absolute.
    let data: IDataObject = match unsafe { SHCreateDataObject(None, Some(&items), None) } {
        Ok(d) => d,
        Err(_) => return false,
    };
    let source: IDropSource = Source.into();

    let mut effect = DROPEFFECT_NONE;
    unsafe {
        // Returns DRAGDROP_S_DROP or DRAGDROP_S_CANCEL; either way the outcome
        // we care about is in `effect`.
        let _ = DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
            &mut effect,
        );
    }
    effect == DROPEFFECT_MOVE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_drag_copies() {
        assert_eq!(effect_for(MODIFIERKEYS_FLAGS(0)), DROPEFFECT_COPY);
    }

    #[test]
    fn shift_moves() {
        assert_eq!(effect_for(MK_SHIFT), DROPEFFECT_MOVE);
    }

    #[test]
    fn ctrl_copies_explicitly() {
        assert_eq!(effect_for(MK_CONTROL), DROPEFFECT_COPY);
    }

    #[test]
    fn ctrl_wins_over_shift() {
        // Both held is ambiguous; the non-destructive reading wins.
        let both = MODIFIERKEYS_FLAGS(MK_CONTROL.0 | MK_SHIFT.0);
        assert_eq!(effect_for(both), DROPEFFECT_COPY);
    }

    #[test]
    fn dragging_nothing_does_nothing() {
        assert!(!start_drag(&[]));
    }
}
