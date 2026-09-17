// What the inspector shows for one file, produced off the UI thread.
//
// An image preview is the shell's own thumbnail, via IShellItemImageFactory —
// the same call Explorer makes, so a RAW file, a PDF or a video gets whatever
// its installed handler produces rather than only what we can decode. It is
// also unavoidably slow: it opens and decodes the file, and for a video seeks
// into it. So this runs on a worker like every other disk read here, and the
// result arrives as a posted message.
//
// A text preview reuses the search module's decoder, which already handles the
// UTF-16 that a lot of Windows text actually is, and already refuses binaries.

use std::collections::{HashMap, VecDeque};

use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
    SIIGBF_THUMBNAILONLY,
};

/// First slice of a text file offered to the inspector. A screenful is the
/// point; anything more is a file the person should open properly.
const TEXT_HEAD_BYTES: u64 = 64 * 1024;

/// What the inspector was able to make of a file.
pub enum Preview {
    /// A shell thumbnail. Owned: the bitmap is deleted when this is dropped.
    Image(HBITMAP),
    Text(String),
    /// What is inside a folder, without going into it: names, folders first,
    /// and how many more there were.
    Folder { names: Vec<String>, more: usize },
    /// Nothing to show beyond the facts the listing already has.
    None,
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Preview::Image(b) = self {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        }
    }
}

/// True when the shell is worth asking for a thumbnail of this extension.
///
/// An allowlist rather than "ask and see": `GetImage` opens and decodes the
/// file to find out, so asking about every .exe and .dll in System32 would be
/// a disk read per row to learn nothing.
///
/// ponytail: a fixed list. It covers what people actually browse; reading the
/// registry for every handler registered under ShellEx\ThumbnailHandler would
/// cover the rest, if anyone misses one.
pub fn has_thumbnail(extension: Option<&str>) -> bool {
    const KINDS: &[&str] = &[
        // images
        "bmp", "gif", "heic", "heif", "ico", "jfif", "jpe", "jpeg", "jpg", "png", "psd", "tif",
        "tiff", "webp", "avif", "dds", "svg", // camera raw
        "arw", "cr2", "cr3", "dng", "nef", "orf", "raf", "rw2", // video
        "avi", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "webm", "wmv", // documents
        "pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx",
    ];
    match extension {
        Some(e) => KINDS.contains(&e.trim_start_matches('.').to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// A bounded map of shell thumbnails, keyed by path and modified time so an
/// edited file re-renders rather than showing what it used to look like.
///
/// ponytail: oldest-first eviction by insertion order, not by use. A grid
/// scrolled back and forth across the boundary re-fetches; the shell has its
/// own thumbnail cache underneath, so re-fetching is cheap. An LRU here would
/// be a second cache in front of a cache.
#[derive(Default)]
pub struct ThumbCache {
    /// None means "asked, and there is nothing" — kept so it is not asked again.
    by_key: HashMap<String, Option<HBITMAP>>,
    order: VecDeque<String>,
}

/// Enough for several screens of a grid at once. Each entry is a bitmap of a
/// few hundred KB, so this is the memory ceiling in disguise.
const MAX_THUMBS: usize = 400;

impl ThumbCache {
    pub fn get(&self, key: &str) -> Option<HBITMAP> {
        self.by_key.get(key).copied().flatten()
    }

    pub fn has(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    pub fn insert(&mut self, key: String, bmp: Option<HBITMAP>) {
        if self.by_key.insert(key.clone(), bmp).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_THUMBS {
            if let Some(old) = self.order.pop_front() {
                if let Some(Some(b)) = self.by_key.remove(&old) {
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(b.0));
                    }
                }
            }
        }
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        for bmp in self.by_key.values().flatten() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bmp.0));
            }
        }
    }
}

/// A grid cell's image: the file's thumbnail if it should have one, and
/// otherwise its file-type icon rendered at the size asked for.
///
/// THUMBNAILONLY is not just about refusing an icon — it is what makes the
/// shell *extract*. Without it a file whose thumbnail is not already cached
/// comes back as its icon straight away, which is why the flag is asked for
/// whenever the extension says there is a thumbnail to be had. For everything
/// else the plain call gives a crisp icon at 72px, rather than the 16px list
/// icon stretched to fit.
///
/// Blocking: call on a worker thread.
pub fn cell_image(path: &str, extension: Option<&str>, edge: i32) -> Option<HBITMAP> {
    unsafe {
        if has_thumbnail(extension) {
            if let Some(bmp) = shell_image(path, edge, true) {
                return Some(bmp);
            }
        }
        shell_image(path, edge, false)
    }
}

/// Build the preview for `path`. Blocking: call on a worker thread.
///
/// `edge` is the longest side wanted, in pixels. The shell is allowed to
/// return something larger (SIIGBF_BIGGERSIZEOK) because a cached thumbnail at
/// the next size up is free, while forcing an exact size re-renders it.
/// Names offered for a folder. A screenful; past that the answer to "what is
/// in here" is to go in.
const PEEK_NAMES: usize = 60;

pub fn build(path: &str, is_dir: bool, extension: Option<&str>, edge: i32) -> Preview {
    if is_dir {
        // Peeking into a folder is a plain directory read, already written and
        // already on a worker thread.
        let Ok(mut entries) = crate::fs::list_dir(path) else {
            return Preview::None;
        };
        entries.retain(|e| !e.is_hidden);
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let more = entries.len().saturating_sub(PEEK_NAMES);
        return Preview::Folder {
            names: entries
                .iter()
                .take(PEEK_NAMES)
                .map(|e| {
                    if e.is_dir {
                        format!("{}\\", e.name)
                    } else {
                        e.name.clone()
                    }
                })
                .collect(),
            more,
        };
    }
    if has_thumbnail(extension) {
        if let Some(bmp) = unsafe { shell_image(path, edge, true) } {
            return Preview::Image(bmp);
        }
        // A handler that failed or a file that is not really what its
        // extension says: fall through and try to read it as text.
    }
    match crate::search::read_text_head(path, TEXT_HEAD_BYTES) {
        Some(text) => Preview::Text(text),
        None => Preview::None,
    }
}

/// `thumbnail_only` refuses a scaled-up file-type icon, which is right for the
/// inspector — a blurry 32px icon across the panel looks like a bug — and
/// wrong for a grid cell, where it is the only sensible thing to draw.
unsafe fn shell_image(path: &str, edge: i32, thumbnail_only: bool) -> Option<HBITMAP> {
    // The worker has no apartment of its own. Shell thumbnail handlers are
    // in-process COM objects and expect one; without this they fail outright.
    // Uninitialised on the way out so a worker thread leaves nothing behind.
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let result = (|| {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR::from_raw(wide.as_ptr()), None).ok()?;
        factory
            .GetImage(
                SIZE {
                    cx: edge,
                    cy: edge,
                },
                if thumbnail_only {
                    SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK
                } else {
                    SIIGBF_BIGGERSIZEOK
                },
            )
            .ok()
    })();
    CoUninitialize();
    result.filter(|b| !b.is_invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_kinds_are_matched_case_and_dot_insensitively() {
        assert!(has_thumbnail(Some(".JPG")));
        assert!(has_thumbnail(Some("mp4")));
        assert!(!has_thumbnail(Some(".rs")));
        assert!(!has_thumbnail(None), "a folder has no extension");
    }

    #[test]
    fn the_cache_evicts_the_oldest_and_remembers_a_miss() {
        let mut c = ThumbCache::default();
        // A None is an answer: "asked, nothing there". It must be remembered,
        // or every repaint asks the shell about the same file again.
        c.insert("a".into(), None);
        assert!(c.has("a"));
        assert!(c.get("a").is_none());

        for i in 0..MAX_THUMBS + 10 {
            c.insert(format!("k{}", i), None);
        }
        assert_eq!(c.order.len(), MAX_THUMBS);
        assert_eq!(c.by_key.len(), MAX_THUMBS);
        assert!(!c.has("a"), "the oldest went first");
        assert!(c.has(&format!("k{}", MAX_THUMBS + 9)), "the newest stayed");

        // Re-inserting a key already held must not grow the order queue, or
        // the cap would evict live entries while the map kept growing.
        let len = c.order.len();
        c.insert(format!("k{}", MAX_THUMBS + 9), None);
        assert_eq!(c.order.len(), len);
    }

    #[test]
    fn a_folder_previews_as_what_is_in_it() {
        let dir = std::env::temp_dir().join("fx-peek-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zeta-folder")).unwrap();
        std::fs::write(dir.join("alpha.txt"), b"x").unwrap();
        let path = dir.to_string_lossy().into_owned();

        match build(&path, true, None, 256) {
            Preview::Folder { ref names, more } => {
                // Folders first whatever they are called, and marked as such.
                assert_eq!(names[..], ["zeta-folder\\".to_string(), "alpha.txt".to_string()]);
                assert_eq!(more, 0);
            }
            _ => panic!("a folder should preview as its contents"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_big_folder_says_how_many_more_there_are() {
        let dir = std::env::temp_dir().join("fx-peek-many");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..PEEK_NAMES + 7 {
            std::fs::write(dir.join(format!("f{:03}.txt", i)), b"x").unwrap();
        }
        match build(&dir.to_string_lossy(), true, None, 256) {
            Preview::Folder { ref names, more } => {
                assert_eq!(names.len(), PEEK_NAMES);
                assert_eq!(more, 7);
            }
            _ => panic!("still a folder"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_text_file_previews_as_its_contents() {
        let dir = std::env::temp_dir().join("fx-preview-test");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("note.txt");
        std::fs::write(&file, b"hello preview").unwrap();
        let path = file.to_string_lossy().into_owned();

        match build(&path, false, Some(".txt"), 256) {
            Preview::Text(ref t) => assert_eq!(t, "hello preview"),
            _ => panic!("a text file should preview as text"),
        }

        // A binary is refused rather than rendered as mojibake.
        let bin = dir.join("blob.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert!(matches!(
            build(&bin.to_string_lossy(), false, Some(".bin"), 256),
            Preview::None
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
