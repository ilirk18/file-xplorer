// Absolute shell item id lists, with the frees taken care of.
//
// Both the shell context menu and drag-and-drop need the same thing: turn a
// list of paths into PIDLs, use them, free them. Getting the free wrong leaks
// on every right-click, so it lives in one place with a Drop impl.

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFindLastID, ILFree, ILGetSize, SHParseDisplayName};

/// A COM apartment for as long as it is held.
///
/// Every function here runs on a worker thread — listing a folder never
/// happens on the UI thread — and a worker has no apartment of its own. The
/// shell's folder objects are in-process COM and simply fail without one,
/// which is a "Cannot open This PC" that looks like a permissions problem.
///
/// `CoUninitialize` is called only when this call is what initialised the
/// apartment: RPC_E_CHANGED_MODE means somebody else's, and balancing their
/// count would tear it down under them.
pub struct Apartment(bool);

impl Apartment {
    pub fn enter() -> Apartment {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Apartment(hr.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// One absolute PIDL, owned as plain bytes.
///
/// A listing holds thousands of entries, gets cloned, sorted and filtered, and
/// crosses to a worker thread — none of which a raw `*mut ITEMIDLIST` survives
/// without a `Clone` and a `Drop` that have to be right every time. An
/// id list is a self-describing byte sequence, so copying it is enough, and a
/// `Vec` gives the clone and the free for nothing.
///
/// Stored as `u16` rather than `u8` because an `ITEMIDLIST` starts with a
/// `u16` length and a byte vector is only aligned to one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pidl(Vec<u16>);

impl Pidl {
    /// Copy an id list the caller owns. Does not take ownership of `raw`.
    ///
    /// # Safety
    /// `raw` must be a valid absolute id list.
    pub unsafe fn copy_from(raw: *const ITEMIDLIST) -> Option<Pidl> {
        if raw.is_null() {
            return None;
        }
        let bytes = ILGetSize(Some(raw)) as usize;
        if bytes == 0 {
            return None;
        }
        // Round up: a trailing odd byte would otherwise be dropped, and the
        // terminator is what tells the shell where the list ends.
        let mut out = vec![0u16; bytes.div_ceil(2)];
        std::ptr::copy_nonoverlapping(
            raw as *const u8,
            out.as_mut_ptr() as *mut u8,
            bytes,
        );
        Some(Pidl(out))
    }

    /// Resolve a path the shell understands, including `shell:` names.
    pub fn from_path(path: &str) -> Option<Pidl> {
        let _com = Apartment::enter();
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut raw: *mut ITEMIDLIST = std::ptr::null_mut();
        unsafe {
            SHParseDisplayName(
                windows::core::PCWSTR::from_raw(wide.as_ptr()),
                None,
                &mut raw,
                0,
                None,
            )
            .ok()?;
            let copied = Pidl::copy_from(raw);
            ILFree(Some(raw));
            copied
        }
    }

    pub fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0.as_ptr() as *const ITEMIDLIST
    }

    /// The last id, i.e. this item relative to its parent folder.
    pub fn child_ptr(&self) -> *const ITEMIDLIST {
        unsafe { ILFindLastID(self.as_ptr()) as *const _ }
    }
}

/// Owns absolute PIDLs and frees them on drop.
pub struct PidlList(Vec<*mut ITEMIDLIST>);

impl PidlList {
    /// Parse each path. Paths the shell cannot resolve are skipped rather than
    /// failing the batch: one bad entry should not cost you the context menu.
    pub fn absolute(paths: &[String]) -> Option<PidlList> {
        if paths.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let wide: Vec<u16> = p.encode_utf16().chain(std::iter::once(0)).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let ok = unsafe {
                SHParseDisplayName(
                    windows::core::PCWSTR::from_raw(wide.as_ptr()),
                    None,
                    &mut pidl,
                    0,
                    None,
                )
            };
            if ok.is_ok() && !pidl.is_null() {
                out.push(pidl);
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(PidlList(out))
        }
    }

    pub fn first(&self) -> *const ITEMIDLIST {
        self.0[0]
    }

    /// The absolute pointers, for APIs that take a NULL parent folder.
    pub fn absolute_ptrs(&self) -> Vec<*const ITEMIDLIST> {
        self.0.iter().map(|p| *p as *const _).collect()
    }
}

impl Drop for PidlList {
    fn drop(&mut self) {
        for p in self.0.drain(..) {
            unsafe { ILFree(Some(p)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_pidl_survives_being_copied_about() {
        let a = Pidl::from_path("C:\\").expect("the C drive resolves");
        assert!(!a.as_ptr().is_null());
        // The point of owning bytes: clone and compare are free and correct,
        // which a raw pointer would not be.
        let b = a.clone();
        assert_eq!(a, b);
        drop(a);
        assert!(!b.as_ptr().is_null(), "the clone owns its own copy");

        // Two spellings of the same place resolve to the same id list.
        let c = Pidl::from_path("C:").expect("C: resolves too");
        assert_eq!(b, c);
    }

    #[test]
    fn a_namespace_name_resolves_where_a_file_path_would_not() {
        assert!(Pidl::from_path("shell:MyComputerFolder").is_some());
        assert!(Pidl::from_path(r"Z:\nowhere\at\all").is_none());
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(PidlList::absolute(&[]).is_none());
    }

    #[test]
    fn unresolvable_paths_are_skipped_not_fatal() {
        let paths = vec![
            "C:\\".to_string(),
            "Z:\\definitely\\not\\a\\real\\path\\at\\all".to_string(),
        ];
        // C:\ resolves, so we still get a list back.
        let list = PidlList::absolute(&paths).expect("C:\\ should resolve");
        assert_eq!(list.absolute_ptrs().len(), 1);
    }

    #[test]
    fn absolute_ptrs_match_the_resolved_count() {
        let list = PidlList::absolute(&["C:\\".to_string()]).unwrap();
        assert_eq!(list.absolute_ptrs().len(), 1);
        assert!(!list.first().is_null());
    }
}
