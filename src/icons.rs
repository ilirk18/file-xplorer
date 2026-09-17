// System icon lookup, cached by file extension.
//
// SHGetFileInfoW with SHGFI_ICON hands back an HICON that the caller owns, so
// the cache destroys every handle it holds when it is dropped. The previous
// version leaked one handle per distinct extension for the life of the process
// (harmless only because nothing ever called it).

use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

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

#[derive(Default)]
pub struct IconCache {
    by_key: HashMap<String, Option<HICON>>,
}

impl IconCache {
    pub fn new() -> Self {
        Self::default()
    }

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

unsafe fn lookup(key: &str) -> Option<HICON> {
    let (path, attrs) = if key == "<dir>" {
        ("C:\\folder".to_string(), FILE_ATTRIBUTE_DIRECTORY)
    } else if key == "<file>" {
        ("C:\\file".to_string(), FILE_ATTRIBUTE_NORMAL)
    } else {
        (format!("C:\\file{}", key), FILE_ATTRIBUTE_NORMAL)
    };

    let w = wide(&path);
    let mut info = SHFILEINFOW::default();
    let ok = SHGetFileInfoW(
        PCWSTR::from_raw(w.as_ptr()),
        attrs,
        Some(&mut info),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
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
    fn repeated_lookups_hit_the_cache() {
        let mut c = IconCache::new();
        c.get("<dir>");
        c.get("<dir>");
        c.get(".txt");
        assert_eq!(c.len(), 2, "one entry per distinct key");
    }
}
