// Browsing inside archives, by shelling out to 7-Zip.
//
// The trick that makes this cheap: an archive is addressed as part of an
// ordinary path. `C:\dl\src.zip\sub\main.rs` is not a path Windows can stat,
// but nothing above `fs::list_any` needs it to be. Breadcrumbs, Back, Up, the
// tab label, type-ahead and the filter all work on the string they already
// had, so entering an archive needed no new state anywhere in the UI.
//
// Reading only. 7-Zip's CLI can write, but a file operation that half succeeds
// inside a container is not something to bolt on, so `ops::Op::touches_archive`
// refuses every destructive operation whose paths lead into one.
//
// ponytail: the listing is a `7z l -slt` per archive, cached by path and
// mtime. If archives ever feel slow it is that process launch, not the parse.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use crate::fs::{self, FileEntry};

/// Extensions we will try to open as folders. 7-Zip reads far more, but these
/// are the ones worth guessing at: opening a `.dll` as an archive because
/// 7-Zip technically can would be a surprise, not a feature.
const ARCHIVE_EXTS: &[&str] = &[
    ".zip", ".7z", ".rar", ".tar", ".gz", ".tgz", ".bz2", ".xz", ".cab", ".iso", ".jar", ".whl",
];

/// Windows hides a console window for a child process only if asked.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn is_archive_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTS.iter().any(|e| lower.ends_with(e))
}

/// Where 7-Zip lives, if it does. Looked up once.
fn seven_zip() -> Option<PathBuf> {
    static FOUND: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
    let mut cache = FOUND.lock().ok()?;
    if let Some(hit) = cache.as_ref() {
        return hit.clone();
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(dir) = std::env::var_os(var) {
            candidates.push(PathBuf::from(dir).join("7-Zip").join("7z.exe"));
        }
    }
    let found = candidates.into_iter().find(|p| p.is_file()).or_else(|| {
        // Not installed in the usual place; it may still be on PATH.
        Command::new("where")
            .arg("7z.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .map(|l| PathBuf::from(l.trim()))
            })
    });
    *cache = Some(found.clone());
    found
}

use std::os::windows::process::CommandExt;

/// Split a path into the archive file and the path inside it.
///
/// Returns None when no component of the path is an archive *file* — a real
/// folder called `backup.zip` is a folder, and checking rather than guessing
/// is what keeps it one.
pub fn split(path: &str) -> Option<(String, String)> {
    // Cheap reject first: the great majority of navigations are not archives
    // and must not cost a metadata call.
    if !path.to_lowercase().contains('.') {
        return None;
    }
    let trimmed = path.trim_end_matches('\\');
    let parts: Vec<&str> = trimmed.split('\\').collect();
    let mut prefix = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            prefix.push('\\');
        }
        prefix.push_str(part);
        if !is_archive_name(part) {
            continue;
        }
        // A drive root is "C:" here and needs its slash back before we can
        // ask the filesystem about it; an archive is never a drive root, so
        // this only matters for correctness of the is_file check below.
        if std::path::Path::new(&prefix).is_file() {
            let inner = parts[i + 1..].join("\\");
            return Some((prefix, inner));
        }
    }
    None
}

/// One entry as 7-Zip reports it.
#[derive(Clone)]
struct Item {
    /// Path relative to the archive root, backslash separated.
    path: String,
    size: u64,
    modified: u64,
    is_dir: bool,
}

/// Archive listings, keyed by archive path. The value carries the archive's
/// own size and mtime so a rebuilt archive is re-read rather than served stale.
static CACHE: Mutex<Option<HashMap<String, (u64, u64, Vec<Item>)>>> = Mutex::new(None);

fn stamp(archive: &str) -> (u64, u64) {
    match std::fs::metadata(archive) {
        Ok(m) => (
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        Err(_) => (0, 0),
    }
}

fn items(archive: &str) -> Result<Vec<Item>, String> {
    let now = stamp(archive);
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some((len, mtime, items)) = map.get(archive) {
            if (*len, *mtime) == now {
                return Ok(items.clone());
            }
        }
    }

    let exe = seven_zip().ok_or_else(|| {
        "7-Zip is not installed. Install it to browse inside archives.".to_string()
    })?;
    let output = Command::new(exe)
        .args(["l", "-slt", "-ba", archive])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not run 7-Zip: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "7-Zip could not read this archive.\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let parsed = parse_listing(&String::from_utf8_lossy(&output.stdout));

    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(archive.to_string(), (now.0, now.1, parsed.clone()));
    }
    Ok(parsed)
}

/// Parse `7z l -slt` output: `Key = Value` lines, one blank-line-separated
/// block per entry.
fn parse_listing(text: &str) -> Vec<Item> {
    let mut out = Vec::new();
    let mut path = String::new();
    let mut size = 0u64;
    let mut modified = 0u64;
    let mut is_dir = false;
    let mut have_path = false;

    let flush = |path: &mut String, size: &mut u64, modified: &mut u64, is_dir: &mut bool, have: &mut bool, out: &mut Vec<Item>| {
        if *have && !path.is_empty() {
            out.push(Item {
                path: std::mem::take(path),
                size: *size,
                modified: *modified,
                is_dir: *is_dir,
            });
        }
        path.clear();
        *size = 0;
        *modified = 0;
        *is_dir = false;
        *have = false;
    };

    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut path, &mut size, &mut modified, &mut is_dir, &mut have_path, &mut out);
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        match key.trim() {
            "Path" => {
                // A second Path without a blank line between means the first
                // block ended early; do not merge them.
                if have_path {
                    flush(&mut path, &mut size, &mut modified, &mut is_dir, &mut have_path, &mut out);
                }
                path = value.trim().replace('/', "\\");
                have_path = true;
            }
            "Size" => size = value.trim().parse().unwrap_or(0),
            "Modified" => modified = filetime_from(value.trim()),
            "Attributes" => is_dir = value.contains('D'),
            "Folder" => is_dir = is_dir || value.trim() == "+",
            _ => {}
        }
    }
    flush(&mut path, &mut size, &mut modified, &mut is_dir, &mut have_path, &mut out);
    out
}

/// `2024-01-31 12:34:56` to FILETIME ticks, so entries inside an archive sort
/// and display the same way real files do.
///
/// The days-from-civil arithmetic is Howard Hinnant's: a calendar is the one
/// thing not to improvise, and it avoids pulling in a date crate for this.
fn filetime_from(s: &str) -> u64 {
    let (date, time) = match s.split_once(' ') {
        Some(p) => p,
        None => (s, "00:00:00"),
    };
    let d: Vec<i64> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    let t: Vec<i64> = time.split(':').filter_map(|p| p.parse().ok()).collect();
    if d.len() != 3 || t.is_empty() {
        return 0;
    }
    let (y, m, day) = (d[0], d[1], d[2]);
    if !(1601..=9999).contains(&y) || !(1..=12).contains(&m) {
        return 0;
    }

    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_1970 = era * 146_097 + doe - 719_468;

    let secs = days_since_1970 * 86_400
        + t.first().copied().unwrap_or(0) * 3600
        + t.get(1).copied().unwrap_or(0) * 60
        + t.get(2).copied().unwrap_or(0);
    // FILETIME counts from 1601; Unix from 1970. 11644473600 seconds between.
    let ticks = (secs + 11_644_473_600) * 10_000_000;
    ticks.max(0) as u64
}

/// List the direct children of `inner` inside `archive`, as ordinary entries.
///
/// Archives commonly list only files, leaving folders implied by their paths,
/// so intermediate folders are synthesised from what is there.
pub fn list(archive: &str, inner: &str) -> Result<Vec<FileEntry>, String> {
    let items = items(archive)?;
    let prefix = if inner.is_empty() {
        String::new()
    } else {
        format!("{}\\", inner.trim_end_matches('\\'))
    };

    let mut files: Vec<FileEntry> = Vec::new();
    let mut folders: std::collections::BTreeSet<String> = Default::default();

    for item in items {
        let Some(rest) = item.path.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('\\') {
            // Something further down: only its first component shows here.
            Some((first, _)) => {
                folders.insert(first.to_string());
            }
            None => {
                if item.is_dir {
                    folders.insert(rest.to_string());
                } else {
                    files.push(FileEntry {
                        extension: fs::extension_of(rest),
                        name: rest.to_string(),
                        size: item.size,
                        modified: item.modified,
                        is_dir: false,
                        is_reparse: false,
                        is_hidden: false,
                        dir_size_known: false,
                        target: None,
                    });
                }
            }
        }
    }

    let mut out: Vec<FileEntry> = folders
        .into_iter()
        .map(|name| FileEntry {
            name,
            size: 0,
            modified: 0,
            is_dir: true,
            is_reparse: false,
            is_hidden: false,
            dir_size_known: false,
            extension: None,
            target: None,
        })
        .collect();
    out.append(&mut files);
    Ok(out)
}

/// Pull one file out to a temporary folder and return where it landed, so the
/// shell can open it. Read-only: edits to the temp copy do not go back in.
pub fn extract_to_temp(archive: &str, inner: &str) -> Result<String, String> {
    let exe = seven_zip().ok_or_else(|| "7-Zip is not installed.".to_string())?;
    let dir = std::env::temp_dir().join("FileXplorer").join("extract");
    let _ = std::fs::create_dir_all(&dir);

    // `e` rather than `x`: flattened, so the result is predictably <dir>\<leaf>.
    let output = Command::new(exe)
        .arg("e")
        .arg(archive)
        .arg(inner)
        .arg(format!("-o{}", dir.display()))
        .arg("-y")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not run 7-Zip: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "7-Zip could not extract that.\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let out = dir.join(fs::path_leaf(inner));
    if out.is_file() {
        Ok(out.to_string_lossy().into_owned())
    } else {
        Err("7-Zip reported success but produced no file.".to_string())
    }
}

/// Extract `entries` (paths relative to the archive root; empty means all of
/// it) into `dest`, keeping their folder structure.
pub fn extract_to(archive: &str, entries: &[String], dest: &str) -> Result<(), String> {
    let exe = seven_zip().ok_or_else(|| "7-Zip is not installed.".to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("x").arg(archive);
    for e in entries {
        cmd.arg(e);
    }
    // -aou renames rather than overwrites: an extract must not quietly replace
    // a file that was already sitting in the destination.
    let output = cmd
        .arg(format!("-o{}", dest))
        .arg("-aou")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not run 7-Zip: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "7-Zip could not extract that.\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Pack `sources` into a new archive. The extension decides the format, which
/// is 7-Zip's own rule.
pub fn create(archive: &str, sources: &[String]) -> Result<(), String> {
    let exe = seven_zip().ok_or_else(|| "7-Zip is not installed.".to_string())?;
    if sources.is_empty() {
        return Err("Nothing selected to archive.".to_string());
    }
    let mut cmd = Command::new(exe);
    // -t is left off on purpose: 7-Zip picks the format from the extension,
    // so naming the file .7z or .zip is the whole of the user interface.
    cmd.arg("a").arg(archive);
    for s in sources {
        cmd.arg(s);
    }
    let output = cmd
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not run 7-Zip: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "7-Zip could not create that archive.\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_not_an_archive() {
        assert!(split("C:\\Users\\me\\Documents").is_none());
        assert!(split("C:\\").is_none());
    }

    #[test]
    fn a_folder_that_merely_ends_in_zip_is_still_a_folder() {
        // The check is "is this component a file", not "does it look like one".
        let base = std::env::temp_dir().join("fx_archive_dir").join("looks.zip");
        let _ = std::fs::create_dir_all(&base);
        let inside = base.join("child");
        let _ = std::fs::create_dir_all(&inside);
        assert!(split(&inside.to_string_lossy()).is_none());
        let _ = std::fs::remove_dir_all(base.parent().unwrap());
    }

    #[test]
    fn an_archive_component_splits_off_the_path_inside_it() {
        let base = std::env::temp_dir().join("fx_archive_split");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let zip = base.join("src.zip");
        std::fs::write(&zip, b"not really a zip, but it is a file").unwrap();

        let deep = format!("{}\\sub\\main.rs", zip.display());
        let (archive, inner) = split(&deep).expect("the zip is a file, so it splits");
        assert_eq!(archive, zip.to_string_lossy());
        assert_eq!(inner, "sub\\main.rs");

        // At the archive root the inner path is empty, not missing.
        let (_, root_inner) = split(&zip.to_string_lossy()).unwrap();
        assert_eq!(root_inner, "");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert!(is_archive_name("Backup.ZIP"));
        assert!(is_archive_name("src.tar.gz"));
        assert!(!is_archive_name("notes.txt"));
    }

    #[test]
    fn a_listing_becomes_entries_one_level_deep() {
        let text = "\
Path = readme.md
Size = 12
Modified = 2024-01-31 12:34:56
Attributes = A

Path = src\\main.rs
Size = 40
Modified = 2024-02-01 00:00:00
Attributes = A

Path = src\\deep\\mod.rs
Size = 7
Attributes = A
";
        let items = parse_listing(text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].size, 12);
        assert!(items[0].modified > 0);

        // Folders are implied by the paths, not listed, and must appear anyway.
        let text_owned = text.to_string();
        let listed = one_level(&parse_listing(&text_owned), "");
        assert_eq!(listed, vec![("src".to_string(), true), ("readme.md".to_string(), false)]);

        let inside = one_level(&parse_listing(&text_owned), "src");
        assert_eq!(inside, vec![("deep".to_string(), true), ("main.rs".to_string(), false)]);
    }

    /// The folder-synthesising half of `list`, without needing a real archive.
    fn one_level(items: &[Item], inner: &str) -> Vec<(String, bool)> {
        let prefix = if inner.is_empty() {
            String::new()
        } else {
            format!("{}\\", inner)
        };
        let mut folders: std::collections::BTreeSet<String> = Default::default();
        let mut files = Vec::new();
        for item in items {
            let Some(rest) = item.path.strip_prefix(prefix.as_str()) else {
                continue;
            };
            match rest.split_once('\\') {
                Some((first, _)) => {
                    folders.insert(first.to_string());
                }
                None if item.is_dir => {
                    folders.insert(rest.to_string());
                }
                None => files.push((rest.to_string(), false)),
            }
        }
        let mut out: Vec<(String, bool)> = folders.into_iter().map(|f| (f, true)).collect();
        out.extend(files);
        out
    }

    #[test]
    fn a_directory_entry_is_recognised_by_its_attributes() {
        let items = parse_listing("Path = folder\nAttributes = D\n\n");
        assert_eq!(items.len(), 1);
        assert!(items[0].is_dir);
    }

    #[test]
    fn a_known_timestamp_converts_to_the_right_filetime() {
        // 1970-01-01 00:00:00 is exactly 11644473600 seconds after the
        // FILETIME epoch.
        assert_eq!(filetime_from("1970-01-01 00:00:00"), 11_644_473_600 * 10_000_000);
        // And a day later is one day later.
        let day = 86_400u64 * 10_000_000;
        assert_eq!(
            filetime_from("1970-01-02 00:00:00") - filetime_from("1970-01-01 00:00:00"),
            day
        );
        // Leap years have to be right or every date after February drifts.
        assert_eq!(
            filetime_from("2024-03-01 00:00:00") - filetime_from("2024-02-28 00:00:00"),
            2 * day
        );
        assert_eq!(
            filetime_from("2023-03-01 00:00:00") - filetime_from("2023-02-28 00:00:00"),
            day
        );
    }

    /// A real archive, built and read back through 7-Zip itself.
    ///
    /// Skipped rather than failed when 7-Zip is not installed: this is the one
    /// part of the app that depends on software we do not ship, and a machine
    /// without it should still get a green test run.
    #[test]
    fn a_real_archive_lists_and_extracts() {
        if seven_zip().is_none() {
            eprintln!("skipped: 7-Zip is not installed");
            return;
        }
        let base = std::env::temp_dir().join("fx_archive_roundtrip");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("payload").join("sub")).unwrap();
        std::fs::write(base.join("payload").join("top.txt"), b"top").unwrap();
        std::fs::write(base.join("payload").join("sub").join("deep.txt"), b"deep!").unwrap();

        let zip = base.join("made.zip");
        let ok = Command::new(seven_zip().unwrap())
            .arg("a")
            .arg(&zip)
            .arg(base.join("payload").join("*"))
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "7-Zip could not create the test archive");
        let zip = zip.to_string_lossy().into_owned();

        // The root lists one file and the folder its children imply.
        let root = list(&zip, "").unwrap();
        let mut names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["sub", "top.txt"]);
        assert!(root.iter().any(|e| e.name == "sub" && e.is_dir));
        let top = root.iter().find(|e| e.name == "top.txt").unwrap();
        assert_eq!(top.size, 3);
        assert!(top.modified > 0, "entries carry their timestamp");

        // And one level down shows only what is there.
        let inside = list(&zip, "sub").unwrap();
        assert_eq!(inside.len(), 1);
        assert_eq!(inside[0].name, "deep.txt");

        // Opening a file pulls a readable copy out.
        let temp = extract_to_temp(&zip, "sub\\deep.txt").unwrap();
        assert_eq!(std::fs::read_to_string(&temp).unwrap(), "deep!");

        // And extracting keeps the structure.
        let dest = base.join("out");
        extract_to(&zip, &["sub\\deep.txt".into()], &dest.to_string_lossy()).unwrap();
        assert!(dest.join("sub").join("deep.txt").is_file());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_unparseable_timestamp_is_zero_rather_than_nonsense() {
        assert_eq!(filetime_from(""), 0);
        assert_eq!(filetime_from("whenever"), 0);
        assert_eq!(filetime_from("0000-00-00 00:00:00"), 0);
    }
}
