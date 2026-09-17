// UI Automation, so a screen reader can read the file list.
//
// Everything this app draws is Direct2D on a bare window, which to a screen
// reader is one rectangle with nothing in it. The modal dialogs are ordinary
// Win32 — the palette's list, the prompts, batch rename — and are already
// readable; the listing is the part that is not, and it is the part that
// matters, so that is what this describes.
//
// The tree is deliberately flat: the window hosts one List, the List holds one
// ListItem per entry. UIA's own host provider supplies everything about the
// window itself, so nothing here has to describe a window.
//
// Providers may be called from another thread, so none of them reach into
// `AppState`. They read a `Snapshot` behind a mutex which the paint path
// refreshes — and only refreshes when something is actually listening, so a
// machine with no screen reader running does none of this work.

// UIA's property and control-type constants are named in the Windows style,
// which the pattern lint objects to. They resolve to constants and so compare
// rather than bind; the lint is about the spelling, and the spelling is not
// ours to change.
#![allow(non_upper_case_globals)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use windows::core::{implement, IUnknownImpl, Interface, Result, BOOL};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::Graphics::Gdi::ClientToScreen;

/// One row, as a screen reader should hear it.
#[derive(Clone, Default)]
pub struct Row {
    pub name: String,
    /// "Folder", or the type, size and date, read after the name.
    pub detail: String,
}

/// What the providers are allowed to know. Refreshed from the paint path.
#[derive(Default)]
pub struct Snapshot {
    /// Bumped by the list itself; used to skip rebuilding `rows` when only the
    /// cursor moved. Rebuilding a hundred thousand names per keystroke is the
    /// obvious way to make this feature unusable.
    pub revision: u64,
    pub folder: String,
    pub rows: Arc<Vec<Row>>,
    pub cursor: Option<usize>,
    pub selected: HashSet<u32>,
    /// The rows area in client pixels, and how entries are laid out in it, so
    /// each item can report where it is on screen.
    pub list: (i32, i32, i32, i32),
    pub line_h: i32,
    pub columns: u32,
    pub scroll_px: i32,
    pub hwnd: isize,
}

impl Snapshot {
    /// Bounding rectangle of one entry, in screen coordinates. Empty when the
    /// entry is scrolled out of view, which is what UIA expects for an item
    /// that exists but is not showing.
    fn item_rect(&self, index: usize) -> UiaRect {
        let (lx, ly, lw, lh) = self.list;
        // A zero column count is the details view before the layout has run
        // once; one column is what it will be.
        let cols = self.columns.max(1) as i32;
        if self.line_h <= 0 || lw <= 0 {
            return UiaRect::default();
        }
        let line = index as i32 / cols;
        let col = index as i32 % cols;
        let y = ly + line * self.line_h - self.scroll_px;
        if y + self.line_h <= ly || y >= ly + lh {
            return UiaRect::default();
        }
        let w = lw / cols;
        let mut pt = POINT {
            x: lx + col * w,
            y,
        };
        unsafe {
            let _ = ClientToScreen(HWND(self.hwnd as *mut _), &mut pt);
        }
        UiaRect {
            left: pt.x as f64,
            top: pt.y as f64,
            width: w as f64,
            height: self.line_h as f64,
        }
    }
}

pub type Shared = Arc<Mutex<Snapshot>>;

/// A runtime id has to be stable for one element and different for every
/// other. UIA prepends its own window part when the first entry is this.
const APPEND: i32 = UiaAppendRuntimeId as i32;

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    ISelectionProvider
)]
pub struct ListProvider {
    shared: Shared,
}

impl ListProvider {
    pub fn new(shared: Shared) -> ListProvider {
        ListProvider { shared }
    }

    fn snapshot(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        // A poisoned lock here would mean a provider panicked mid-read. The
        // data is a plain snapshot, so carrying on with it is strictly better
        // than taking the process down inside a screen reader's call.
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn item(&self, index: usize) -> IRawElementProviderSimple {
        ItemProvider {
            shared: self.shared.clone(),
            index,
        }
        .into()
    }
}

impl IRawElementProviderSimple_Impl for ListProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<windows::core::IUnknown> {
        if id == UIA_SelectionPatternId {
            return Ok(self.to_interface());
        }
        Err(windows::core::Error::empty())
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let snap = self.snapshot();
        Ok(match id {
            UIA_ControlTypePropertyId => VARIANT::from(UIA_ListControlTypeId.0),
            // The folder being shown: the first thing worth hearing on arrival.
            UIA_NamePropertyId => VARIANT::from(snap.folder.as_str()),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => {
                VARIANT::from(true)
            }
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(true),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // Everything about the window — its name, its bounds, that it is a
        // window at all — is the host's to answer, not ours.
        unsafe { UiaHostProviderFromHwnd(HWND(self.snapshot().hwnd as *mut _)) }
    }
}

impl IRawElementProviderFragment_Impl for ListProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let snap = self.snapshot();
        let last = snap.rows.len().checked_sub(1);
        drop(snap);
        let target = match direction {
            NavigateDirection_FirstChild => Some(0),
            NavigateDirection_LastChild => last,
            // The window above us is the host's business, and we have no
            // siblings: there is one list in this tree.
            _ => None,
        };
        match target {
            Some(i) => self.item(i).cast(),
            None => Err(windows::core::Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
        safearray(&[APPEND, 1])
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let snap = self.snapshot();
        let (x, y, w, h) = snap.list;
        let mut pt = POINT { x, y };
        unsafe {
            let _ = ClientToScreen(HWND(snap.hwnd as *mut _), &mut pt);
        }
        Ok(UiaRect {
            left: pt.x as f64,
            top: pt.y as f64,
            width: w as f64,
            height: h as f64,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for ListProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let snap = self.snapshot();
        let count = snap.rows.len();
        // Linear over what is on screen rather than clever: only visible items
        // have a rectangle at all, and a hit test happens once per mouse stop.
        for i in 0..count {
            let r = snap.item_rect(i);
            if r.width > 0.0
                && x >= r.left
                && x < r.left + r.width
                && y >= r.top
                && y < r.top + r.height
            {
                drop(snap);
                return self.item(i).cast();
            }
        }
        drop(snap);
        Ok(self.to_interface())
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let cursor = self.snapshot().cursor;
        match cursor {
            Some(i) => self.item(i).cast(),
            None => Err(windows::core::Error::empty()),
        }
    }
}

impl ISelectionProvider_Impl for ListProvider_Impl {
    fn GetSelection(&self) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
        let snap = self.snapshot();
        let mut chosen: Vec<u32> = snap.selected.iter().copied().collect();
        chosen.sort_unstable();
        drop(snap);
        let providers: Vec<IRawElementProviderSimple> = chosen
            .into_iter()
            .map(|i| self.item(i as usize))
            .collect();
        provider_array(&providers)
    }

    fn CanSelectMultiple(&self) -> Result<BOOL> {
        Ok(true.into())
    }

    fn IsSelectionRequired(&self) -> Result<BOOL> {
        // Escape clears the selection, so an empty one is a real state.
        Ok(false.into())
    }
}

// ---------------------------------------------------------------------------
// One entry
// ---------------------------------------------------------------------------

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    ISelectionItemProvider
)]
pub struct ItemProvider {
    shared: Shared,
    index: usize,
}

impl ItemProvider {
    fn snapshot(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sibling(&self, index: usize) -> IRawElementProviderSimple {
        ItemProvider {
            shared: self.shared.clone(),
            index,
        }
        .into()
    }
}

impl IRawElementProviderSimple_Impl for ItemProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<windows::core::IUnknown> {
        if id == UIA_SelectionItemPatternId {
            return Ok(self.to_interface());
        }
        Err(windows::core::Error::empty())
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let snap = self.snapshot();
        let row = snap.rows.get(self.index);
        Ok(match id {
            UIA_ControlTypePropertyId => VARIANT::from(UIA_ListItemControlTypeId.0),
            UIA_NamePropertyId => {
                VARIANT::from(row.map(|r| r.name.as_str()).unwrap_or_default())
            }
            // Read after the name, so "report.txt" comes first and "TXT file,
            // 12 KB, yesterday" follows rather than burying it.
            UIA_ItemStatusPropertyId | UIA_HelpTextPropertyId => {
                VARIANT::from(row.map(|r| r.detail.as_str()).unwrap_or_default())
            }
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => {
                VARIANT::from(true)
            }
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(true),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(snap.cursor == Some(self.index)),
            UIA_IsOffscreenPropertyId => VARIANT::from(snap.item_rect(self.index).width <= 0.0),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // An item is ours alone; only the root is hosted by the window.
        Err(windows::core::Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for ItemProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let snap = self.snapshot();
        let count = snap.rows.len();
        drop(snap);
        let target = match direction {
            NavigateDirection_Parent => {
                let list: IRawElementProviderSimple =
                    ListProvider::new(self.shared.clone()).into();
                return list.cast();
            }
            NavigateDirection_NextSibling if self.index + 1 < count => Some(self.index + 1),
            NavigateDirection_PreviousSibling if self.index > 0 => Some(self.index - 1),
            _ => None,
        };
        match target {
            Some(i) => self.sibling(i).cast(),
            None => Err(windows::core::Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
        safearray(&[APPEND, 2, self.index as i32])
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        Ok(self.snapshot().item_rect(self.index))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        // Moving the cursor is the keyboard's job and would mean calling back
        // into the UI thread from whatever thread UIA used.
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        let list: IRawElementProviderSimple = ListProvider::new(self.shared.clone()).into();
        list.cast()
    }
}

impl ISelectionItemProvider_Impl for ItemProvider_Impl {
    fn Select(&self) -> Result<()> {
        Ok(())
    }

    fn AddToSelection(&self) -> Result<()> {
        Ok(())
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        Ok(())
    }

    fn IsSelected(&self) -> Result<BOOL> {
        Ok(self.snapshot().selected.contains(&(self.index as u32)).into())
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        Ok(ListProvider::new(self.shared.clone()).into())
    }
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

fn safearray(values: &[i32]) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
    use windows::Win32::System::Com::SAFEARRAY;
    use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
    use windows::Win32::System::Variant::VT_I4;

    unsafe {
        let array: *mut SAFEARRAY = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
        if array.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_OUTOFMEMORY,
            ));
        }
        for (i, v) in values.iter().enumerate() {
            let index = i as i32;
            SafeArrayPutElement(array, &index, v as *const i32 as *const _)?;
        }
        Ok(array)
    }
}

fn provider_array(
    providers: &[IRawElementProviderSimple],
) -> Result<*mut windows::Win32::System::Com::SAFEARRAY> {
    use windows::core::Interface;
    use windows::Win32::System::Com::SAFEARRAY;
    use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
    use windows::Win32::System::Variant::VT_UNKNOWN;

    unsafe {
        let array: *mut SAFEARRAY = SafeArrayCreateVector(VT_UNKNOWN, 0, providers.len() as u32);
        if array.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_OUTOFMEMORY,
            ));
        }
        for (i, p) in providers.iter().enumerate() {
            let index = i as i32;
            SafeArrayPutElement(array, &index, p.as_raw())?;
        }
        Ok(array)
    }
}

/// True when a screen reader — or any other automation client — is attached.
///
/// Everything this module costs is behind this: with nothing listening the
/// snapshot is never built, and a folder of a hundred thousand files does not
/// pay for a feature nobody in the room is using.
pub fn clients_are_listening() -> bool {
    unsafe { UiaClientsAreListening().as_bool() }
}

/// Answer `WM_GETOBJECT` for the automation tree.
///
/// Returns None for the object ids we do not serve — MSAA's, the caret's —
/// which the window procedure passes to `DefWindowProc` as it always did.
pub fn on_get_object(
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    shared: &Shared,
) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    let provider: IRawElementProviderSimple = ListProvider::new(shared.clone()).into();
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider) })
}

/// Tell any listening client that the cursor has moved to `index`.
pub fn announce_focus(shared: &Shared, index: usize) {
    if !clients_are_listening() {
        return;
    }
    let provider: IRawElementProviderSimple = ItemProvider {
        shared: shared.clone(),
        index,
    }
    .into();
    unsafe {
        let _ = UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId);
    }
}

/// Tell any listening client that the listing itself changed.
pub fn announce_listing(shared: &Shared) {
    if !clients_are_listening() {
        return;
    }
    let provider: IRawElementProviderSimple = ListProvider::new(shared.clone()).into();
    unsafe {
        let _ = UiaRaiseAutomationEvent(&provider, UIA_StructureChangedEventId);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(rows: usize, line_h: i32, columns: u32, scroll_px: i32) -> Snapshot {
        Snapshot {
            rows: Arc::new(vec![Row::default(); rows]),
            list: (0, 100, 400, 200),
            line_h,
            columns,
            scroll_px,
            ..Default::default()
        }
    }

    #[test]
    fn an_offscreen_item_has_no_rectangle() {
        let s = snap(100, 20, 1, 0);
        // Ten rows fit in 200 pixels starting at y=100.
        assert!(s.item_rect(0).height > 0.0, "the first row is on screen");
        assert!(s.item_rect(9).height > 0.0, "the last visible row is too");
        assert_eq!(
            s.item_rect(50).width,
            0.0,
            "a row far below the viewport reports nothing, which is how UIA \
             says an item exists but is not showing"
        );
    }

    #[test]
    fn scrolling_moves_which_items_have_rectangles() {
        let mut s = snap(100, 20, 1, 0);
        assert!(s.item_rect(0).height > 0.0);
        s.scroll_px = 20 * 40;
        assert_eq!(s.item_rect(0).width, 0.0, "scrolled off the top");
        assert!(s.item_rect(40).height > 0.0, "and this one scrolled in");
    }

    #[test]
    fn a_grid_puts_several_items_on_one_line() {
        let s = snap(100, 120, 4, 0);
        let first = s.item_rect(0);
        let second = s.item_rect(1);
        assert_eq!(first.top, second.top, "same line");
        assert_eq!(first.left + first.width, second.left, "side by side");
        // The fifth is on the next line down.
        assert_eq!(s.item_rect(4).top, first.top + 120.0);
    }

    #[test]
    fn degenerate_geometry_never_divides_by_zero() {
        // No line height: nothing has a place yet.
        assert_eq!(snap(10, 0, 1, 0).item_rect(0).width, 0.0);
        // No column count: treated as the one column the details view has,
        // rather than dividing the width by nothing.
        assert_eq!(snap(10, 20, 0, 0).item_rect(0).width, 400.0);
    }
}
