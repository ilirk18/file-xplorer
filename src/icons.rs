// System icon lookup, cached by file extension.
//
// SHGetFileInfoW with SHGFI_ICON hands back an HICON that the caller owns, so
// the cache destroys every handle it holds when it is dropped. The previous
// version leaked one handle per distinct extension for the life of the process
// (harmless only because nothing ever called it).

use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::UI::Shell::{ExtractIconExW, PathIsNetworkPathW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_PIDL, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

use crate::fs::wide;

/// Cache key for a listing entry. Directories all share one icon; files are
/// keyed by lowercased extension, so a folder of 10,000 .jpg files costs one
/// shell lookup rather than 10,000.
pub fn icon_key(extension: Option<&str>, is_dir: bool) -> String {
    if is_dir {
        "<dir>".to_string()
    } else {
        match extension {
            Some(e) if !e.is_empty() => e.to_ascii_lowercase(),
            _ => "<file>".to_string(),
        }
    }
}

/// Cache key for one specific path, used by the sidebar so Downloads, Pictures
/// and each drive get their real shell icon instead of a generic folder.
/// Only ever applied to the handful of sidebar entries, and cached, so the
/// disk access it implies happens once per location.
pub fn icon_key_for_path(path: &str) -> String {
    format!("{}{}", PATH_PREFIX, path.to_lowercase())
}

const PATH_PREFIX: &str = "path:";

/// Cache key for an icon named the way Windows names one: `shell32.dll,4`,
/// or a bare `.ico`/`.exe` path, whose index is 0.
pub fn custom_icon_key(spec: &str) -> String {
    format!("{}{}", ICON_PREFIX, spec.to_lowercase())
}

const ICON_PREFIX: &str = "icon:";

#[derive(Default)]
pub struct IconCache {
    by_key: HashMap<String, Option<HICON>>,
}

impl IconCache {
    /// Small (16px) shell icon for a cache key from `icon_key`.
    ///
    /// Looks the icon up from a synthetic path plus explicit attributes
    /// (SHGFI_USEFILEATTRIBUTES), so nothing touches the disk and the call
    /// cannot block on a slow volume.
    pub fn get(&mut self, key: &str) -> Option<HICON> {
        if let Some(cached) = self.by_key.get(key) {
            return *cached;
        }
        let icon = unsafe { lookup(key) };
        self.by_key.insert(key.to_string(), icon);
        icon
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }
}

/// One icon out of a file that holds some: a DLL, an EXE, or an `.ico`.
///
/// `ExtractIconExW` is the whole feature — it takes the same `file,index`
/// pair the shell has always used, searches for a bare name the way
/// `LoadLibrary` does, and hands back a handle this cache already knows how to
/// destroy.
///
/// Its return value is not a success flag and is not treated as one: a missing
/// file gives `0xFFFFFFFF`, and index `-1` gives the *count* of icons in the
/// file without filling anything in. Both leave the handle null, so the handle
/// is what decides.
unsafe fn extract_icon(spec: &str) -> Option<HICON> {
    let (file, index) = match spec.rsplit_once(',') {
        Some((file, index)) => (file.trim(), index.trim().parse().ok()?),
        // No comma: the file holds one icon, or we want its first.
        None => (spec.trim(), 0i32),
    };
    if file.is_empty() {
        return None;
    }
    let w = wide(file);
    let mut small = HICON::default();
    ExtractIconExW(PCWSTR::from_raw(w.as_ptr()), index, None, Some(&mut small), 1);
    (!small.is_invalid()).then_some(small)
}

unsafe fn lookup(key: &str) -> Option<HICON> {
    // A "path:" key asks for the icon of a real location, so the shell is
    // allowed to touch it. Everything else is resolved from attributes alone
    // and never hits the disk.
    if let Some(spec) = key.strip_prefix(ICON_PREFIX) {
        return extract_icon(spec);
    }
    let (path, attrs, flags) = if let Some(real) = key.strip_prefix(PATH_PREFIX) {
        // A shell location has no file to ask about, so it is resolved to an
        // id list first. Worth the extra call for the three of them: the
        // Recycle Bin's icon is the one that says whether it is full.
        if crate::shellns::is_shell_path(real) {
            return icon_for_shell_path(real);
        }
        // A real location is a disk read, and on an unreachable share that
        // read blocks until SMB gives up \u2014 on the paint path, since this is
        // what draws the sidebar.
        //
        // ponytail: a network place gets the generic folder icon. Asking for
        // the real one on a worker and filling it in later is the upgrade, and
        // it is the thumbnail cache's shape; nobody has missed the icon yet.
        let w = wide(real);
        if PathIsNetworkPathW(PCWSTR::from_raw(w.as_ptr())).as_bool() {
            return lookup("<dir>");
        }
        (
            real.to_string(),
            FILE_ATTRIBUTE_NORMAL,
            SHGFI_ICON | SHGFI_SMALLICON,
        )
    } else if key == "<dir>" {
        (
            "C:\\folder".to_string(),
            FILE_ATTRIBUTE_DIRECTORY,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        )
    } else if key == "<file>" {
        (
            "C:\\file".to_string(),
            FILE_ATTRIBUTE_NORMAL,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        )
    } else {
        (
            format!("C:\\file{}", key),
            FILE_ATTRIBUTE_NORMAL,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        )
    };

    let w = wide(&path);
    let mut info = SHFILEINFOW::default();
    let ok = SHGetFileInfoW(
        PCWSTR::from_raw(w.as_ptr()),
        attrs,
        Some(&mut info),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        flags,
    );
    if ok == 0 || info.hIcon.is_invalid() {
        None
    } else {
        Some(info.hIcon)
    }
}

/// The icon of a namespace location, by way of its id list.
unsafe fn icon_for_shell_path(path: &str) -> Option<HICON> {
    let pidls = crate::pidl::PidlList::absolute(&[path.to_string()])?;
    let mut info = SHFILEINFOW::default();
    let ok = SHGetFileInfoW(
        PCWSTR(pidls.first() as *const u16),
        FILE_ATTRIBUTE_NORMAL,
        Some(&mut info),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_SMALLICON | SHGFI_PIDL,
    );
    if ok == 0 || info.hIcon.is_invalid() {
        None
    } else {
        Some(info.hIcon)
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        for icon in self.by_key.values().flatten() {
            unsafe {
                let _ = DestroyIcon(*icon);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_share_one_key() {
        assert_eq!(icon_key(Some(".txt"), true), icon_key(None, true));
    }

    #[test]
    fn extensions_are_case_folded() {
        assert_eq!(icon_key(Some(".TXT"), false), icon_key(Some(".txt"), false));
    }

    #[test]
    fn extensionless_files_share_a_key() {
        assert_eq!(icon_key(None, false), "<file>");
        assert_eq!(icon_key(Some(""), false), "<file>");
    }

    #[test]
    fn path_keys_are_distinct_and_case_folded() {
        assert_ne!(icon_key_for_path("C:\\Users"), icon_key(None, true));
        assert_eq!(
            icon_key_for_path("C:\\Users"),
            icon_key_for_path("c:\\users")
        );
    }

    #[test]
    fn repeated_lookups_hit_the_cache() {
        let mut c = IconCache::default();
        c.get("<dir>");
        c.get("<dir>");
        c.get(".txt");
        assert_eq!(c.len(), 2, "one entry per distinct key");
    }
}
