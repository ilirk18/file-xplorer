// Recursive name search.
//
// Walks a tree on a worker thread and streams matches back in batches, so a
// search over a large folder fills in progressively instead of freezing until
// it finishes.
//
// The walk reuses `fs::list_dir` rather than re-implementing enumeration: it
// already handles long paths, skips `.`/`..`, and fills in every field a
// `FileEntry` needs. Each match keeps its path *relative to the search root* in
// `name`, which means the rest of the app opens a result with the same
// `path_join(current_path, name)` it uses for an ordinary listing — no special
// case anywhere downstream.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::fs::{self, FileEntry};

/// Results are flushed at this many, so the pane starts filling immediately.
const BATCH: usize = 128;
/// A hard ceiling. Past this, a search is not an answer, it is a listing.
const MAX_RESULTS: usize = 20_000;
/// Files larger than this are not read for a content search. Grepping a
/// multi-gigabyte disk image is not a search, it is a stall.
const MAX_CONTENT_BYTES: u64 = 8 * 1024 * 1024;

/// What to look for.
#[derive(Clone, Debug, Default)]
pub struct Query {
    /// Name pattern, `*` allowed. Empty matches every name.
    pub name: String,
    /// Extensions to keep, lowercased and without the dot. Empty keeps every
    /// file. A non-empty list also drops folders, which have no extension.
    pub exts: Vec<String>,
    /// Text that must appear inside the file. None searches names only.
    pub content: Option<String>,
}

/// Pull `ext:` tokens out of a query, so `ext:rs,toml render` reads as "render,
/// in Rust and TOML files". Everything else stays in the name pattern.
fn split_ext_filter(query: &str) -> (String, Vec<String>) {
    let mut exts = Vec::new();
    let mut rest = Vec::new();
    for word in query.split_whitespace() {
        match word.strip_prefix("ext:") {
            Some(list) => exts.extend(
                list.split(',')
                    .map(|e| e.trim_start_matches('.').to_lowercase())
                    .filter(|e| !e.is_empty()),
            ),
            None => rest.push(word),
        }
    }
    (rest.join(" "), exts)
}

impl Query {
    pub fn by_name(pattern: &str) -> Query {
        let (name, exts) = split_ext_filter(pattern);
        Query {
            name,
            exts,
            content: None,
        }
    }

    pub fn by_content(text: &str) -> Query {
        Query {
            name: String::new(),
            exts: Vec::new(),
            content: Some(text.to_string()),
        }
    }

    /// True when `entry` passes the extension filter.
    pub fn ext_ok(&self, extension: Option<&str>) -> bool {
        if self.exts.is_empty() {
            return true;
        }
        match extension {
            Some(ext) => {
                let ext = ext.trim_start_matches('.');
                self.exts.iter().any(|x| x == ext)
            }
            None => false,
        }
    }

    /// What the tab calls itself.
    pub fn label(&self) -> String {
        let name = if self.exts.is_empty() {
            self.name.clone()
        } else if self.name.is_empty() {
            format!("*.{}", self.exts.join(", *."))
        } else {
            format!("{} (*.{})", self.name, self.exts.join(", *."))
        };
        match (&self.content, name.is_empty()) {
            (Some(text), true) => format!("Containing: {}", text),
            (Some(text), false) => format!("{} containing: {}", name, text),
            (None, _) => format!("Search: {}", name),
        }
    }
}

/// True when the file at `path` contains `needle`, ignoring case.
///
/// Binary files are skipped rather than matched by accident: a NUL byte early
/// on is the same heuristic every grep uses, and it is right far more often
/// than any amount of content sniffing would be. UTF-16 is decoded properly
/// because that is what a lot of Windows text actually is, and a UTF-8 reader
/// sees every second byte as a NUL and calls the file binary.
pub fn file_contains(path: &str, needle: &str) -> bool {
    let Some(text) = read_text(path, MAX_CONTENT_BYTES) else {
        return false;
    };
    // Lowercasing the whole file allocates a second copy of it, which at 8 MiB
    // is a transient the allocator will not notice. A streaming
    // case-insensitive search would be a lot of code for a search that is
    // already dominated by disk time.
    text.to_lowercase().contains(needle)
}

/// Read up to `limit` bytes of `path` as text, or None if it is not text.
///
/// The BOM cases are decoded properly because a lot of Windows text is UTF-16,
/// and a UTF-8 reader sees every second byte as a NUL and calls the file
/// binary. Everything else is UTF-8 unless an early NUL says otherwise, which
/// is the same heuristic every grep uses and is right far more often than any
/// amount of content sniffing would be.
pub fn read_text(path: &str, limit: u64) -> Option<String> {
    use std::io::Read;

    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > limit {
        return None;
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path).ok()?.read_to_end(&mut bytes).ok()?;
    decode_text(&bytes)
}

/// Read at most `limit` bytes from the *front* of `path` as text.
///
/// Unlike `read_text` a file larger than the limit is not refused, it is
/// truncated: a preview of the first screenful of a 2 GB log is useful, and
/// searching it for a word is not.
pub fn read_text_head(path: &str, limit: u64) -> Option<String> {
    use std::io::Read;

    let f = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    f.take(limit).read_to_end(&mut bytes).ok()?;
    // A truncated UTF-16 file can end mid-unit; the decoder drops the odd byte.
    decode_text(&bytes)
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    Some(match bytes.get(..2) {
        Some([0xFF, 0xFE]) => decode_utf16(&bytes[2..], true),
        Some([0xFE, 0xFF]) => decode_utf16(&bytes[2..], false),
        _ => {
            if bytes.iter().take(8192).any(|b| *b == 0) {
                return None; // binary
            }
            String::from_utf8_lossy(bytes).into_owned()
        }
    })
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

pub struct Search {
    pub cancel: Arc<AtomicBool>,
}

impl Search {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Shorter than this, a subsequence match means nothing: "rs" is a subsequence
/// of half the names on a disk. Substring matching still applies at any length.
const MIN_FUZZY: usize = 3;

/// True when `name` matches `query`.
///
/// Case-insensitive substring, with `*` treated as a gap so `*.rs` and
/// `test*.txt` do what people expect. No full glob: `?` and character classes
/// are not worth the parser for a name filter.
///
/// Without a `*`, a substring miss falls back to the palette's subsequence
/// matcher, so "gtd" finds "get_tree_depth.rs" the same way it finds a command.
pub fn matches(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    let query = query.to_lowercase();

    if !query.contains('*') {
        return name.contains(&query)
            || (query.chars().count() >= MIN_FUZZY
                && crate::palette::score(&name, &query).is_some());
    }

    // Anchor the ends only when the pattern does not start/end with `*`.
    let anchored_start = !query.starts_with('*');
    let anchored_end = !query.ends_with('*');
    let parts: Vec<&str> = query.split('*').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        let Some(found) = name[pos..].find(part) else {
            return false;
        };
        let abs = pos + found;
        if i == 0 && anchored_start && abs != 0 {
            return false;
        }
        pos = abs + part.len();
    }
    if anchored_end && pos != name.len() {
        return false;
    }
    true
}

/// One flush of results.
pub struct Batch {
    pub generation: u64,
    pub entries: Vec<FileEntry>,
    pub done: bool,
}

/// Walk `root` for names matching `query`, handing each batch to `deliver`.
/// Blocking: call on a worker thread.
pub fn run<F>(
    root: String,
    query: Query,
    generation: u64,
    cancel: Arc<AtomicBool>,
    mut deliver: F,
) where
    F: FnMut(Batch),
{
    // Lowercased once here rather than per file.
    let needle = query.content.as_ref().map(|t| t.to_lowercase());
    let mut pending: Vec<FileEntry> = Vec::with_capacity(BATCH);
    let mut total = 0usize;
    // Relative directories still to visit. Iterative, so depth costs heap.
    let mut stack: Vec<String> = vec![String::new()];

    while let Some(rel) = stack.pop() {
        if cancel.load(Ordering::Relaxed) || total >= MAX_RESULTS {
            break;
        }
        let abs = if rel.is_empty() {
            root.clone()
        } else {
            fs::path_join(&root, &rel)
        };
        let Ok(entries) = fs::list_dir(&abs) else {
            continue; // Unreadable folder: skip it, keep searching.
        };

        for e in entries {
            let rel_name = if rel.is_empty() {
                e.name.clone()
            } else {
                fs::path_join(&rel, &e.name)
            };
            // Never descend a junction or symlink: that is how a walker ends up
            // looping, or counting the same tree under two names.
            if e.is_dir && !e.is_reparse {
                stack.push(rel_name.clone());
            }
            // Name and extension first: both are free, and they narrow what
            // has to be read.
            let hit = query.ext_ok(e.extension.as_deref())
                && matches(&e.name, &query.name)
                && match &needle {
                    // A folder has no contents to search, so a content search
                    // never lists one.
                    Some(n) => !e.is_dir && file_contains(&fs::path_join(&abs, &e.name), n),
                    None => true,
                };
            if hit {
                pending.push(FileEntry {
                    name: rel_name,
                    ..e
                });
                total += 1;
                if pending.len() >= BATCH {
                    deliver(Batch {
                        generation,
                        entries: std::mem::take(&mut pending),
                        done: false,
                    });
                }
                if total >= MAX_RESULTS {
                    break;
                }
            }
        }
    }

    deliver(Batch {
        generation,
        entries: pending,
        done: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_filter_splits_out_of_the_query() {
        let q = Query::by_name("ext:rs,.TOML render");
        assert_eq!(q.name, "render");
        assert_eq!(q.exts, vec!["rs", "toml"]);
        assert!(q.ext_ok(Some(".rs")));
        assert!(q.ext_ok(Some("toml")));
        assert!(!q.ext_ok(Some(".txt")));
        assert!(!q.ext_ok(None), "a folder has no extension to match");
    }

    #[test]
    fn no_ext_filter_keeps_everything() {
        let q = Query::by_name("render");
        assert!(q.exts.is_empty());
        assert!(q.ext_ok(None));
        assert!(q.ext_ok(Some(".exe")));
    }

    #[test]
    fn fuzzy_needs_three_characters() {
        assert!(matches("get_tree_depth.rs", "gtd"));
        assert!(!matches("get_tree_depth.rs", "gx"), "two chars stay literal");
        assert!(matches("get_tree_depth.rs", "tree"), "substring still wins");
    }

    #[test]
    fn glob_is_not_fuzzy() {
        assert!(matches("main.rs", "*.rs"));
        assert!(!matches("main.rs", "*.txt"));
    }

    #[test]
    fn substring_is_case_insensitive() {
        assert!(matches("ReadMe.MD", "readme"));
        assert!(matches("ReadMe.MD", "DME"));
        assert!(!matches("ReadMe.MD", "zzz"));
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches("anything", ""));
    }

    #[test]
    fn star_suffix_matches_extension() {
        assert!(matches("main.rs", "*.rs"));
        assert!(!matches("main.rss", "*.rs"));
        assert!(!matches("main.txt", "*.rs"));
    }

    #[test]
    fn star_prefix_anchors_the_start() {
        assert!(matches("test_one.txt", "test*"));
        assert!(!matches("one_test.txt", "test*"));
    }

    #[test]
    fn star_in_the_middle_spans_a_gap() {
        assert!(matches("test_alpha.txt", "test*.txt"));
        assert!(matches("test.txt", "test*.txt"));
        assert!(!matches("best_alpha.txt", "test*.txt"));
    }

    #[test]
    fn bare_star_matches_everything() {
        assert!(matches("whatever", "*"));
        assert!(matches("", "*"));
    }

    #[test]
    fn walk_finds_nested_matches_with_relative_names() {
        let base = std::env::temp_dir().join("fx_search_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub").join("deeper")).unwrap();
        std::fs::write(base.join("top.rs"), b"").unwrap();
        std::fs::write(base.join("sub").join("mid.rs"), b"").unwrap();
        std::fs::write(base.join("sub").join("deeper").join("low.rs"), b"").unwrap();
        std::fs::write(base.join("ignore.txt"), b"").unwrap();

        let mut found: Vec<String> = Vec::new();
        run(
            base.to_string_lossy().into_owned(),
            Query::by_name("*.rs"),
            1,
            Arc::new(AtomicBool::new(false)),
            |b| found.extend(b.entries.into_iter().map(|e| e.name)),
        );
        found.sort();
        assert_eq!(
            found,
            vec![
                "sub\\deeper\\low.rs".to_string(),
                "sub\\mid.rs".to_string(),
                "top.rs".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_last_batch_is_always_marked_done() {
        let mut dones = 0;
        run(
            "C:\\definitely\\not\\here".into(),
            Query::by_name("x"),
            7,
            Arc::new(AtomicBool::new(false)),
            |b| {
                assert_eq!(b.generation, 7);
                if b.done {
                    dones += 1;
                }
            },
        );
        assert_eq!(dones, 1, "exactly one terminating batch");
    }

    #[test]
    fn cancelling_stops_the_walk() {
        let cancel = Arc::new(AtomicBool::new(true));
        let mut entries = 0;
        run(
            std::env::temp_dir().to_string_lossy().into_owned(),
            Query::default(),
            1,
            cancel,
            |b| entries += b.entries.len(),
        );
        assert_eq!(entries, 0);
    }

    #[test]
    fn content_search_reads_utf8_and_ignores_case() {
        let base = std::env::temp_dir().join("fx_content_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let hit = base.join("hit.txt");
        std::fs::write(&hit, b"the Needle is here").unwrap();
        assert!(file_contains(hit.to_str().unwrap(), "needle"));
        assert!(!file_contains(hit.to_str().unwrap(), "haystack"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_utf16_file_is_decoded_rather_than_called_binary() {
        // Notepad still writes these, and byte-wise they look binary.
        let base = std::env::temp_dir().join("fx_content_utf16");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let p = base.join("wide.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "hello world".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&p, &bytes).unwrap();
        assert!(file_contains(p.to_str().unwrap(), "world"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_binary_file_is_skipped_not_matched() {
        let base = std::env::temp_dir().join("fx_content_binary");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let p = base.join("blob.bin");
        std::fs::write(&p, b"text\x00\x01\x02more text").unwrap();
        assert!(!file_contains(p.to_str().unwrap(), "more"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_content_search_never_lists_a_folder() {
        let base = std::env::temp_dir().join("fx_content_walk");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("yes.txt"), b"alpha beta").unwrap();
        std::fs::write(base.join("no.txt"), b"gamma").unwrap();
        std::fs::write(base.join("sub").join("deep.txt"), b"BETA again").unwrap();

        let mut found: Vec<String> = Vec::new();
        run(
            base.to_string_lossy().into_owned(),
            Query::by_content("beta"),
            1,
            Arc::new(AtomicBool::new(false)),
            |b| found.extend(b.entries.into_iter().map(|e| e.name)),
        );
        found.sort();
        assert_eq!(found, vec!["sub\\deep.txt".to_string(), "yes.txt".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn name_and_content_must_both_match() {
        let base = std::env::temp_dir().join("fx_content_both");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("keep.rs"), b"needle").unwrap();
        std::fs::write(base.join("skip.txt"), b"needle").unwrap();

        let mut found: Vec<String> = Vec::new();
        run(
            base.to_string_lossy().into_owned(),
            Query {
                name: "*.rs".into(),
                content: Some("needle".into()),
                ..Default::default()
            },
            1,
            Arc::new(AtomicBool::new(false)),
            |b| found.extend(b.entries.into_iter().map(|e| e.name)),
        );
        assert_eq!(found, vec!["keep.rs".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_query_says_what_it_is_looking_for() {
        assert_eq!(Query::by_name("*.rs").label(), "Search: *.rs");
        assert_eq!(Query::by_content("todo").label(), "Containing: todo");
        assert_eq!(
            Query {
                name: "*.rs".into(),
                content: Some("todo".into()),
                ..Default::default()
            }
            .label(),
            "*.rs containing: todo"
        );
    }
}
