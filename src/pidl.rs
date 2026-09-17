// Absolute shell item id lists, with the frees taken care of.
//
// Both the shell context menu and drag-and-drop need the same thing: turn a
// list of paths into PIDLs, use them, free them. Getting the free wrong leaks
// on every right-click, so it lives in one place with a Drop impl.

use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFindLastID, ILFree, SHParseDisplayName};

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

    /// The last id of each, i.e. child PIDLs relative to their shared parent.
    /// These borrow from `self`, so the list must outlive them.
    pub fn child_ptrs(&self) -> Vec<*const ITEMIDLIST> {
        self.0
            .iter()
            .map(|p| unsafe { ILFindLastID(*p) } as *const _)
            .collect()
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
    fn child_and_absolute_counts_match() {
        let list = PidlList::absolute(&["C:\\".to_string()]).unwrap();
        assert_eq!(list.child_ptrs().len(), list.absolute_ptrs().len());
    }
}
