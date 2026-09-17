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

/// True when `name` matches `query`.
///
/// Case-insensitive substring, with `*` treated as a gap so `*.rs` and
/// `test*.txt` do what people expect. No full glob: `?` and character classes
/// are not worth the parser for a name filter.
pub fn matches(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    let query = query.to_lowercase();

    if !query.contains('*') {
        return name.contains(&query);
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
    query: String,
    generation: u64,
    cancel: Arc<AtomicBool>,
    mut deliver: F,
) where
    F: FnMut(Batch),
{
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
            if matches(&e.name, &query) {
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
            "*.rs".into(),
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
            "x".into(),
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
            "".into(),
            1,
            cancel,
            |b| entries += b.entries.len(),
        );
        assert_eq!(entries, 0);
    }
}
