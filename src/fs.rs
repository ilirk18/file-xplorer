// Directory enumeration, path helpers, and value formatting.
//
// Nothing here mutates the filesystem: destructive work goes through `ops`,
// which drives the shell's IFileOperation so we inherit the Recycle Bin,
// progress UI, and conflict prompts instead of reimplementing them.

use std::path::{Component, Path};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{FILETIME, MAX_PATH, SYSTEMTIME};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Time::FileTimeToSystemTime;

/// One file or directory in a listing.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    /// FILETIME as u64 (100-ns ticks since 1601) for sorting; 0 if unavailable.
    pub modified: u64,
    pub is_dir: bool,
    /// Junction, symlink, or other reparse point. Listed, but never recursed into.
    pub is_reparse: bool,
    pub is_hidden: bool,
    /// True once a folder's size has been calculated on demand. Until then a
    /// directory shows no size at all, because guessing one would be a lie.
    pub dir_size_known: bool,
    /// Lowercased, with the leading dot (".txt"). None for directories and
    /// extensionless files. Doubles as the icon-cache key.
    pub extension: Option<String>,
}

impl FileEntry {
    /// Display string for the Size column. Directories show nothing until
    /// their size has been calculated.
    pub fn size_display(&self) -> String {
        if self.is_dir && !self.dir_size_known {
            String::new()
        } else {
            format_size(self.size)
        }
    }

    /// Display string for the Type column.
    ///
    /// Derived from the extension rather than asked of the shell: SHGetFileInfo
    /// with SHGFI_TYPENAME would give "Rust Source File" where this gives "RS
    /// file", but it is a registry lookup per extension and this column exists
    /// to be sorted on, not read aloud.
    pub fn type_display(&self) -> String {
        if self.is_dir {
            return "File folder".to_string();
        }
        match self.extension.as_deref() {
            // The extension already carries its dot.
            Some(ext) if ext.len() > 1 => format!("{} file", ext[1..].to_uppercase()),
            _ => "File".to_string(),
        }
    }

    /// Display string for the Date Modified column, in local time.
    pub fn date_display(&self) -> String {
        format_filetime(self.modified)
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Prefix a path with the extended-length marker so it bypasses MAX_PATH.
///
/// Only worth doing for absolute paths that are actually long; the prefix also
/// disables `.`/`..` normalisation, which is fine because every path we hold is
/// already fully resolved.
fn to_extended(path: &str) -> String {
    if path.starts_with("\\\\?\\") || path.len() < MAX_PATH as usize {
        return path.to_string();
    }
    if let Some(unc) = path.strip_prefix("\\\\") {
        format!("\\\\?\\UNC\\{}", unc)
    } else if is_absolute_drive_path(path) {
        format!("\\\\?\\{}", path)
    } else {
        path.to_string()
    }
}

fn is_absolute_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

fn filetime_to_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

fn decode_wide_cstr(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// List directory contents. Blocking: callers run this on a worker thread.
/// Expand `%VAR%` the way a shell would, so a typed path can use them.
/// Unknown names are left alone rather than blanked, which is what tells you
/// you mistyped one.
pub fn expand_env(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// True when two files do not hold the same bytes.
///
/// Size first, because a different size is a different file and costs one
/// metadata call. Only then are both read. A file we cannot open counts as
/// differing: "I could not tell" must not render as "these are the same".
///
/// ponytail: SipHash over the contents rather than a byte-by-byte compare.
/// One pass per file instead of interleaved reads, and a 64-bit collision on
/// two files of identical length is not a thing that happens by accident.
pub fn files_differ(a: &str, b: &str) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return true;
    };
    if ma.len() != mb.len() {
        return true;
    }
    match (hash_file(a), hash_file(b)) {
        (Some(x), Some(y)) => x != y,
        _ => true,
    }
}

fn hash_file(path: &str) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::Read;

    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        buf[..n].hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// Lowercased extension with its leading dot, or None. Also the icon-cache key.
pub fn extension_of(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| format!(".{}", s.to_ascii_lowercase()))
}

/// Read a directory, or the inside of an archive when the path leads into one.
///
/// Every caller that used to call `list_dir` on a user-chosen path goes
/// through here, which is the whole of what makes archives browsable: the path
/// string carries the archive, so navigation, breadcrumbs and history need no
/// idea that one is involved.
pub fn list_any(path: &str) -> Result<Vec<FileEntry>, String> {
    if let Some((archive, inner)) = crate::archive::split(path) {
        return crate::archive::list(&archive, &inner);
    }
    list_dir(path).map_err(|e| e.to_string())
}

pub fn list_dir(path: &str) -> Result<Vec<FileEntry>, std::io::Error> {
    let extended = to_extended(path);
    let base = extended.trim_end_matches('\\');
    let search = to_wide(&format!("{}\\*", base));

    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileW(PCWSTR::from_raw(search.as_ptr()), &mut data)
            .map_err(|_| std::io::Error::last_os_error())?
    };

    let mut entries = Vec::new();
    loop {
        let name = decode_wide_cstr(&data.cFileName);
        if name != "." && name != ".." {
            let attrs = data.dwFileAttributes;
            let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
            let extension = if is_dir { None } else { extension_of(&name) };
            entries.push(FileEntry {
                name,
                size: ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64),
                modified: filetime_to_u64(&data.ftLastWriteTime),
                is_dir,
                is_reparse: (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0,
                is_hidden: (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0
                    || (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0,
                dir_size_known: false,
                extension,
            });
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    let _ = unsafe { FindClose(handle) };
    Ok(entries)
}

/// Current working directory, for the initial listing.
pub fn current_directory() -> Result<String, std::io::Error> {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned())
}

/// Parent directory, or None at a root (`C:\`, `\\server\share`).
pub fn path_parent(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .map(|par| par.to_string_lossy().into_owned())
}

/// Join a directory with a child name.
pub fn path_join(path: &str, name: &str) -> String {
    Path::new(path).join(name).to_string_lossy().into_owned()
}

/// Final component of a path, for tab labels. Roots keep their full spelling.
pub fn path_leaf(path: &str) -> String {
    path_segments(path)
        .last()
        .cloned()
        .unwrap_or_else(|| path.to_string())
}

/// Breadcrumb segments. The drive prefix and its root slash collapse into one
/// segment, so `C:\Users\Foo` yields `["C:\", "Users", "Foo"]` rather than
/// leaking a bare `\` crumb between the drive and the first folder.
pub fn path_segments(path: &str) -> Vec<String> {
    let p = Path::new(path);
    let mut out: Vec<String> = Vec::new();
    let mut comps = p.components().peekable();

    let mut root = String::new();
    if let Some(Component::Prefix(prefix)) = comps.peek() {
        root.push_str(&prefix.as_os_str().to_string_lossy());
        comps.next();
    }
    if let Some(Component::RootDir) = comps.peek() {
        root.push('\\');
        comps.next();
    }
    if !root.is_empty() {
        out.push(root);
    }

    for comp in comps {
        match comp {
            Component::Normal(s) => out.push(s.to_string_lossy().into_owned()),
            Component::ParentDir => out.push("..".to_string()),
            _ => {}
        }
    }

    if out.is_empty() && !path.is_empty() {
        out.push(path.to_string());
    }
    out
}

/// Rebuild a path from `segments[0..=up_to_index]`, for a breadcrumb click.
pub fn path_from_segments(segments: &[String], up_to_index: usize) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let end = (up_to_index + 1).min(segments.len());
    let mut p = segments[0].clone();
    for seg in &segments[1..end] {
        p = path_join(&p, seg);
    }
    p
}

/// Why a proposed name is not allowed.
#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// Contains a path separator, a drive colon, or a wildcard.
    IllegalCharacter(char),
    /// `.` or `..`.
    DotName,
    /// CON, PRN, AUX, NUL, COM1-9, LPT1-9 — reserved by Win32 whatever the extension.
    ReservedDeviceName,
    /// Windows strips these silently, producing a name the user did not ask for.
    TrailingDotOrSpace,
    TooLong,
}

const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate a bare file name. Rejects anything that would let a rename escape
/// the current directory, which the old `path_join`-based rename allowed.
pub fn validate_file_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::DotName);
    }
    if name.chars().count() > 255 {
        return Err(NameError::TooLong);
    }
    for ch in name.chars() {
        if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (ch as u32) < 0x20
        {
            return Err(NameError::IllegalCharacter(ch));
        }
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(NameError::TrailingDotOrSpace);
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str()) {
        return Err(NameError::ReservedDeviceName);
    }
    Ok(())
}

impl NameError {
    pub fn message(&self) -> String {
        match self {
            NameError::Empty => "The name cannot be empty.".into(),
            NameError::IllegalCharacter(c) if (*c as u32) < 0x20 => {
                "The name cannot contain control characters.".into()
            }
            NameError::IllegalCharacter(_) => {
                "A name cannot contain any of:   \\  /  :  *  ?  \"  <  >  |".into()
            }
            NameError::DotName => "\".\" and \"..\" are reserved names.".into(),
            NameError::ReservedDeviceName => {
                "That is a reserved Windows device name (CON, PRN, AUX, NUL, COM1-9, LPT1-9)."
                    .into()
            }
            NameError::TrailingDotOrSpace => {
                "A name cannot end with a dot or a space - Windows would silently strip it.".into()
            }
            NameError::TooLong => "The name is too long (255 characters maximum).".into(),
        }
    }
}

/// A mounted volume, for the drive sidebar.
#[derive(Clone, Debug)]
pub struct Drive {
    /// Root path including the trailing slash, e.g. `C:\`.
    pub root: String,
    /// Volume label, or a sensible fallback like "Local Disk".
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

impl Drive {
    /// `Windows (C:)`, matching Explorer's own formatting.
    pub fn display(&self) -> String {
        format!("{} ({})", self.label, self.root.trim_end_matches('\\'))
    }

    /// Fraction of the volume in use, for the sidebar capacity bar.
    /// None when the size is unknown, so the bar is simply not drawn.
    pub fn used_fraction(&self) -> Option<f32> {
        if self.total_bytes == 0 {
            return None;
        }
        let used = self.total_bytes.saturating_sub(self.free_bytes);
        Some((used as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32)
    }

    /// `284 GB free of 931 GB`, shown under the label.
    pub fn capacity_text(&self) -> String {
        if self.total_bytes == 0 {
            return String::new();
        }
        format!(
            "{} free of {}",
            format_size(self.free_bytes),
            format_size(self.total_bytes)
        )
    }
}

const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// Enumerate mounted drives. Skips volumes that are not ready (empty card
/// readers, disconnected shares) so the sidebar has no dead entries.
pub fn drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let wide = to_wide(&root);

        let kind = unsafe { GetDriveTypeW(PCWSTR::from_raw(wide.as_ptr())) };
        if kind == DRIVE_NO_ROOT_DIR {
            continue;
        }

        let mut name_buf = [0u16; 261];
        let readable = unsafe {
            GetVolumeInformationW(
                PCWSTR::from_raw(wide.as_ptr()),
                Some(&mut name_buf),
                None,
                None,
                None,
                None,
            )
        }
        .is_ok();
        if !readable && matches!(kind, DRIVE_REMOVABLE | DRIVE_CDROM) {
            continue; // No media inserted.
        }

        let label = {
            let l = decode_wide_cstr(&name_buf);
            if !l.is_empty() {
                l
            } else {
                match kind {
                    DRIVE_REMOVABLE => "Removable Disk".to_string(),
                    DRIVE_REMOTE => "Network Drive".to_string(),
                    DRIVE_CDROM => "CD Drive".to_string(),
                    _ => "Local Disk".to_string(),
                }
            }
        };
        // Capacity is best-effort: a volume can refuse to report it, and that
        // should cost us a bar, not the whole sidebar entry.
        let (mut total_bytes, mut free_bytes) = (0u64, 0u64);
        let mut avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        if unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR::from_raw(wide.as_ptr()),
                Some(&mut avail),
                Some(&mut total),
                Some(&mut free),
            )
        }
        .is_ok()
        {
            total_bytes = total;
            // Report the space this user can actually use, not the raw free
            // space, so a quota-limited volume reads honestly.
            free_bytes = avail.min(free.max(avail));
        }

        out.push(Drive {
            root,
            label,
            total_bytes,
            free_bytes,
        });
    }
    out
}

/// Total bytes under `root`, following no reparse points.
///
/// Iterative rather than recursive: a deep tree should cost heap, not stack.
/// Junctions and symlinks are skipped outright — following them is how a
/// directory walker ends up counting C:\ twice or looping forever.
pub fn dir_size(root: &str) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue; // Unreadable subtree: count what we can and move on.
        };
        for entry in read.flatten() {
            // file_type() comes from the directory entry itself and does not
            // follow the link, which is exactly what we want here.
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if let Ok(md) = entry.metadata() {
                total = total.saturating_add(md.len());
            }
        }
    }
    total
}

/// Human-readable byte count.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    match bytes {
        b if b < KB => format!("{} B", b),
        b if b < MB => format!("{:.0} KB", b as f64 / KB as f64),
        b if b < GB => format!("{:.1} MB", b as f64 / MB as f64),
        b if b < TB => format!("{:.2} GB", b as f64 / GB as f64),
        b => format!("{:.2} TB", b as f64 / TB as f64),
    }
}

/// FILETIME ticks to a local-time `YYYY-MM-DD HH:MM` string. Empty when zero.
pub fn format_filetime(ticks: u64) -> String {
    if ticks == 0 {
        return String::new();
    }
    let ft = FILETIME {
        dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local = FILETIME::default();
    if unsafe { FileTimeToLocalFileTime(&ft, &mut local) }.is_err() {
        return String::new();
    }
    let mut st = SYSTEMTIME::default();
    if unsafe { FileTimeToSystemTime(&local, &mut st) }.is_err() {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_parent() {
        assert_eq!(path_parent("C:\\a\\b"), Some("C:\\a".to_string()));
        assert_eq!(path_parent("C:\\a"), Some("C:\\".to_string()));
        assert_eq!(path_parent("C:\\"), None);
    }

    #[test]
    fn test_path_join() {
        assert_eq!(path_join("C:\\", "a"), "C:\\a");
        assert_eq!(path_join("C:\\a", "b"), "C:\\a\\b");
    }

    #[test]
    fn test_path_segments_collapses_root() {
        // The drive prefix and root slash must be one crumb, not "C:" then "\".
        assert_eq!(
            path_segments("C:\\Users\\Foo"),
            vec!["C:\\".to_string(), "Users".to_string(), "Foo".to_string()]
        );
        assert_eq!(path_segments("C:\\"), vec!["C:\\".to_string()]);
    }

    #[test]
    fn test_path_segments_unc() {
        let segs = path_segments("\\\\server\\share\\dir");
        assert_eq!(segs.len(), 2);
        assert!(segs[0].starts_with("\\\\server\\share"));
        assert_eq!(segs[1], "dir");
    }

    #[test]
    fn test_path_from_segments_roundtrips() {
        for path in ["C:\\", "C:\\Users", "C:\\Users\\Foo\\Bar"] {
            let segs = path_segments(path);
            let rebuilt = path_from_segments(&segs, segs.len() - 1);
            assert_eq!(rebuilt, path, "roundtrip failed for {}", path);
        }
    }

    #[test]
    fn test_path_from_segments_prefix() {
        let segs = path_segments("C:\\Users\\Foo");
        assert_eq!(path_from_segments(&segs, 0), "C:\\");
        assert_eq!(path_from_segments(&segs, 1), "C:\\Users");
        assert_eq!(path_from_segments(&segs, 2), "C:\\Users\\Foo");
    }

    #[test]
    fn test_validate_file_name_rejects_traversal() {
        // The old rename accepted all of these and relocated the file.
        assert!(validate_file_name("..\\evil").is_err());
        assert!(validate_file_name("C:\\evil").is_err());
        assert!(validate_file_name("a/b").is_err());
        assert_eq!(validate_file_name(".."), Err(NameError::DotName));
        assert_eq!(validate_file_name("."), Err(NameError::DotName));
    }

    #[test]
    fn test_validate_file_name_rejects_reserved() {
        assert_eq!(validate_file_name("CON"), Err(NameError::ReservedDeviceName));
        assert_eq!(
            validate_file_name("nul.txt"),
            Err(NameError::ReservedDeviceName)
        );
        assert_eq!(
            validate_file_name("LPT9"),
            Err(NameError::ReservedDeviceName)
        );
    }

    #[test]
    fn test_validate_file_name_rejects_trailing() {
        assert_eq!(
            validate_file_name("report."),
            Err(NameError::TrailingDotOrSpace)
        );
        assert_eq!(
            validate_file_name("report "),
            Err(NameError::TrailingDotOrSpace)
        );
    }

    #[test]
    fn test_validate_file_name_accepts_normal() {
        assert!(validate_file_name("report.txt").is_ok());
        assert!(validate_file_name("My Folder").is_ok());
        assert!(validate_file_name(".gitignore").is_ok());
        assert!(validate_file_name("cafe - notes.md").is_ok());
    }

    #[test]
    fn dir_size_counts_a_real_tree() {
        let base = std::env::temp_dir().join("fx_dir_size_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("sub").join("b.bin"), vec![0u8; 250]).unwrap();

        assert_eq!(dir_size(&base.to_string_lossy()), 350);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dir_size_of_a_missing_path_is_zero() {
        assert_eq!(dir_size("C:\\definitely\\not\\here\\at\\all"), 0);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn test_to_extended_only_prefixes_long_paths() {
        assert_eq!(to_extended("C:\\short"), "C:\\short");
        let long = format!("C:\\{}", "a".repeat(300));
        assert!(to_extended(&long).starts_with("\\\\?\\C:\\"));
        let already = format!("\\\\?\\C:\\{}", "a".repeat(300));
        assert_eq!(to_extended(&already), already);
    }

    #[test]
    fn test_path_leaf() {
        assert_eq!(path_leaf("C:\\Users\\Foo"), "Foo");
        assert_eq!(path_leaf("C:\\"), "C:\\");
    }

    fn drive(total: u64, free: u64) -> Drive {
        Drive {
            root: "C:\\".into(),
            label: "Windows".into(),
            total_bytes: total,
            free_bytes: free,
        }
    }

    #[test]
    fn drive_display_matches_explorer() {
        assert_eq!(drive(0, 0).display(), "Windows (C:)");
    }

    #[test]
    fn used_fraction_is_none_when_size_is_unknown() {
        assert_eq!(drive(0, 0).used_fraction(), None);
    }

    #[test]
    fn used_fraction_is_the_filled_portion() {
        let d = drive(1000, 250);
        assert!((d.used_fraction().unwrap() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn used_fraction_clamps_when_free_exceeds_total() {
        // Quota-limited volumes can report nonsense; never draw a negative bar.
        let d = drive(1000, 5000);
        assert_eq!(d.used_fraction(), Some(0.0));
    }

    #[test]
    fn capacity_text_reads_naturally() {
        let d = drive(2048, 1024);
        assert_eq!(d.capacity_text(), "1 KB free of 2 KB");
        assert_eq!(drive(0, 0).capacity_text(), "");
    }

    #[test]
    fn env_vars_expand_and_unknown_ones_stay_put() {
        std::env::set_var("FX_TEST_DIR", "C:\\somewhere");
        assert_eq!(expand_env("%FX_TEST_DIR%\\sub"), "C:\\somewhere\\sub");
        // A name that is not set is left visible, which is how you see the typo.
        assert_eq!(expand_env("%FX_NOT_SET%\\x"), "%FX_NOT_SET%\\x");
        // An unmatched % is literal, not the start of something.
        assert_eq!(expand_env("50% done"), "50% done");
        assert_eq!(expand_env("plain"), "plain");
    }

    #[test]
    fn files_differ_by_size_then_by_content() {
        let base = std::env::temp_dir().join("fx_differ");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let a = base.join("a").to_string_lossy().into_owned();
        let b = base.join("b").to_string_lossy().into_owned();
        let c = base.join("c").to_string_lossy().into_owned();
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"hello").unwrap();
        std::fs::write(&c, b"world").unwrap();

        assert!(!files_differ(&a, &b));
        assert!(files_differ(&a, &c), "same length, different bytes");
        std::fs::write(&c, b"hello there").unwrap();
        assert!(files_differ(&a, &c), "different length");
        // A file that is not there cannot be called identical.
        assert!(files_differ(&a, &base.join("missing").to_string_lossy()));
        let _ = std::fs::remove_dir_all(&base);
    }
}
