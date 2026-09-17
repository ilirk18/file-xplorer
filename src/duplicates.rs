// Finding files with identical contents, anywhere under one folder.
//
// Content comparison already existed in two places: `fs::files_differ` answers
// it for a pair, and compare mode answers it for two panes side by side. Both
// need to be told which files to look at. This answers the question nobody can
// point at the answer to — *which files anywhere under here are copies of each
// other* — and it is the same comparison, driven by a walk instead of by a
// pairing.
//
// The walk is search's: `fs::list_dir` per directory, an explicit stack rather
// than recursion, and reparse points listed but never descended, because that
// is how a walker ends up counting one tree under two names.
//
// **Size first, bytes second.** Two files of different lengths cannot be
// copies, and a length costs nothing — it came back with the listing. So the
// walk buckets by size and only a bucket holding two or more files is ever
// read. On a real tree that is a small fraction of it, and it is the whole
// reason this is affordable: without it, finding duplicates means reading
// every byte on the disk.
//
// Within a bucket, files are hashed rather than compared pairwise. `files_differ`
// already made that trade for a pair and documents why; here it is not a trade
// but a necessity, since pairwise comparison inside a bucket of n files is
// n²/2 comparisons and hashing is n reads.
//
// Two things this deliberately does not report:
//
//  * **Empty files.** Every zero-byte file is byte-identical to every other
//    one, which is true, useless, and on most disks would bury the findings
//    that matter under hundreds of them.
//  * **Folders.** A folder has no contents of its own to compare. Two folders
//    holding the same files are a real thing to want to find, and a different
//    feature: it is a comparison of listings, not of bytes.
//
// Known limitation: two hard links to one file are reported as duplicates.
// They *are* byte-identical, so the finding is not wrong, but deleting one
// reclaims nothing. Telling them apart means opening every candidate to read
// its file index, which is a handle per file to correct a case that is rare
// outside of system folders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::fs::{self, FileEntry};

/// A hard ceiling on the walk. Past this the answer would not fit on a screen
/// or in memory, and a duplicate finder that has to be waited out is not one
/// anybody uses. Reported as truncated rather than silently cut.
const MAX_FILES: usize = 200_000;
/// A ceiling on findings, for the same reason `search` has one.
const MAX_GROUPS: usize = 5_000;

/// Files that hold the same bytes. Always two or more; a group of one is not a
/// finding.
pub struct Group {
    /// What every file in the group weighs. The group wastes `size * (n - 1)`.
    pub size: u64,
    /// Each file's path *relative to the scanned root*, in `name`, exactly as
    /// a search result carries it — so opening one needs no special case.
    pub entries: Vec<FileEntry>,
}

impl Group {
    /// What deleting all but one copy would give back.
    pub fn reclaimable(&self) -> u64 {
        self.size * (self.entries.len() as u64 - 1)
    }
}

/// One flush of findings.
pub struct Batch {
    pub generation: u64,
    pub groups: Vec<Group>,
    pub done: bool,
    /// Set on the final batch when the walk hit `MAX_FILES` and stopped early,
    /// so the caller can say so rather than implying it found everything.
    pub truncated: bool,
}

/// Findings are flushed at this many groups, so a long scan fills in as it goes
/// rather than landing all at once at the end.
const BATCH: usize = 32;

/// Walk `root`, and hand `deliver` each batch of duplicate groups.
/// Blocking: call on a worker thread.
pub fn run<F>(root: String, generation: u64, cancel: Arc<AtomicBool>, mut deliver: F)
where
    F: FnMut(Batch),
{
    let cancelled = || cancel.load(Ordering::Relaxed);

    // -- phase one: the walk. Bucket every file by the one property that is
    // free to know and that copies must share.
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    let mut files = 0usize;
    let mut truncated = false;
    let mut stack: Vec<String> = vec![String::new()];

    while let Some(rel) = stack.pop() {
        if cancelled() || files >= MAX_FILES {
            truncated = files >= MAX_FILES;
            break;
        }
        let abs = if rel.is_empty() {
            root.clone()
        } else {
            fs::path_join(&root, &rel)
        };
        let Ok(entries) = fs::list_dir(&abs) else {
            continue; // Unreadable folder: skip it, keep walking.
        };
        for e in entries {
            let rel_name = if rel.is_empty() {
                e.name.clone()
            } else {
                fs::path_join(&rel, &e.name)
            };
            if e.is_dir {
                if !e.is_reparse {
                    stack.push(rel_name);
                }
                continue;
            }
            // A reparse point that is a file is a link to bytes somewhere
            // else. Following it would report the target as a copy of itself.
            if e.is_reparse || e.size == 0 {
                continue;
            }
            by_size.entry(e.size).or_default().push(FileEntry {
                name: rel_name,
                ..e
            });
            files += 1;
        }
    }

    // -- phase two: read only what could possibly be a copy.
    //
    // Biggest first, because the biggest duplicates are both the ones worth
    // deleting and the ones worth seeing before the scan is over.
    let mut sizes: Vec<u64> = by_size
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, _)| *k)
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));

    let mut pending: Vec<Group> = Vec::new();
    let mut found = 0usize;

    for size in sizes {
        if cancelled() || found >= MAX_GROUPS {
            break;
        }
        let candidates = by_size.remove(&size).unwrap_or_default();
        let mut by_hash: HashMap<u64, Vec<FileEntry>> = HashMap::new();
        for e in candidates {
            if cancelled() {
                break;
            }
            // A file we cannot read is not a file we can call a copy of
            // anything. `files_differ` makes the same call for the same reason.
            if let Some(h) = fs::hash_file(&fs::path_join(&root, &e.name)) {
                by_hash.entry(h).or_default().push(e);
            }
        }
        // Within one size, group order follows the name of the first member,
        // so two scans of an unchanged tree report the same thing in the same
        // order. A HashMap's own order is not an order.
        let mut groups: Vec<Vec<FileEntry>> =
            by_hash.into_values().filter(|v| v.len() > 1).collect();
        groups.sort_by(|a, b| crate::file_list::natural_cmp(&a[0].name, &b[0].name));

        for mut entries in groups {
            entries.sort_by(|a, b| crate::file_list::natural_cmp(&a.name, &b.name));
            pending.push(Group { size, entries });
            found += 1;
            if pending.len() >= BATCH {
                deliver(Batch {
                    generation,
                    groups: std::mem::take(&mut pending),
                    done: false,
                    truncated: false,
                });
            }
            if found >= MAX_GROUPS {
                break;
            }
        }
    }

    deliver(Batch {
        generation,
        groups: pending,
        done: true,
        truncated,
    });
}

/// Flatten a batch's groups into listing rows, plus which group each row is in.
///
/// `base` is how many groups the listing already holds. Numbering has to
/// continue from it rather than restart per batch: a scan streams, and the
/// group number is what the left-edge bar alternates on, so restarting it
/// every batch would draw two neighbouring groups in one colour at every
/// batch boundary — which reads as one group.
pub fn rows(groups: Vec<Group>, base: u32) -> (Vec<FileEntry>, Vec<(String, u32)>) {
    let mut entries = Vec::new();
    let mut map = Vec::new();
    for (i, g) in groups.into_iter().enumerate() {
        for e in g.entries {
            map.push((e.name.clone(), base + i as u32));
            entries.push(e);
        }
    }
    (entries, map)
}

/// What the footer says when a scan finishes.
///
/// Lives here rather than in the message handler so it can be tested: the
/// wording is the whole result of the feature for anyone whose tree turns out
/// to be clean, and "0 groups" as an answer to "find duplicates" is a worse
/// sentence than the one it replaces.
pub fn summary(groups: u32, files: usize, bytes: u64, truncated: bool) -> String {
    let scope = if truncated {
        " (stopped early: too many files)"
    } else {
        ""
    };
    if groups == 0 {
        return format!("No duplicate files found{}", scope);
    }
    let copies = files as u32 - groups; // One of each group is the one you keep.
    format!(
        "{} group{} of duplicates, {} redundant cop{}, {} reclaimable{}",
        groups,
        if groups == 1 { "" } else { "s" },
        copies,
        if copies == 1 { "y" } else { "ies" },
        fs::format_size(bytes),
        scope
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tree and scan it, returning each group as a sorted list of
    /// relative names, groups ordered as the scan reported them.
    fn scan(base: &std::path::Path) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        run(
            base.to_string_lossy().into_owned(),
            1,
            Arc::new(AtomicBool::new(false)),
            |b| {
                out.extend(
                    b.groups
                        .into_iter()
                        .map(|g| g.entries.into_iter().map(|e| e.name).collect()),
                );
            },
        );
        out
    }

    fn tree(name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&base);
        for (rel, bytes) in files {
            let p = base.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
        }
        base
    }

    #[test]
    fn copies_are_found_across_the_tree_and_named_relative_to_its_root() {
        let base = tree(
            "fx_dupes_basic",
            &[
                ("a.txt", b"the same bytes"),
                (r"sub\b.txt", b"the same bytes"),
                (r"sub\deeper\c.txt", b"the same bytes"),
                ("alone.txt", b"nobody else has this"),
            ],
        );
        let groups = scan(&base);
        assert_eq!(groups.len(), 1, "one finding, not three");
        assert_eq!(
            groups[0],
            vec![
                "a.txt".to_string(),
                r"sub\b.txt".to_string(),
                r"sub\deeper\c.txt".to_string(),
            ],
            "every copy, by a path that joins to the scan root"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn same_length_different_bytes_is_not_a_duplicate() {
        // The case the size bucket exists to narrow and must not decide.
        let base = tree(
            "fx_dupes_collide",
            &[("a.bin", b"aaaaaaaa"), ("b.bin", b"bbbbbbbb")],
        );
        assert!(scan(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn two_separate_groups_of_one_size_stay_separate() {
        let base = tree(
            "fx_dupes_two_groups",
            &[
                ("a1.bin", b"aaaaaaaa"),
                ("a2.bin", b"aaaaaaaa"),
                ("b1.bin", b"bbbbbbbb"),
                ("b2.bin", b"bbbbbbbb"),
            ],
        );
        let mut groups = scan(&base);
        groups.sort();
        assert_eq!(
            groups,
            vec![
                vec!["a1.bin".to_string(), "a2.bin".to_string()],
                vec!["b1.bin".to_string(), "b2.bin".to_string()],
            ],
            "one size, two findings"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_files_are_not_reported_as_copies_of_each_other() {
        // True, and useless: it would bury every real finding.
        let base = tree(
            "fx_dupes_empty",
            &[("one.txt", b""), ("two.txt", b""), ("three.txt", b"")],
        );
        assert!(scan(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn folders_are_never_a_finding() {
        let base = tree("fx_dupes_dirs", &[(r"x\keep.txt", b"x")]);
        std::fs::create_dir_all(base.join("empty_a")).unwrap();
        std::fs::create_dir_all(base.join("empty_b")).unwrap();
        assert!(scan(&base).is_empty(), "two empty folders are not copies");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_group_reports_what_deleting_the_extras_would_give_back() {
        let g = Group {
            size: 1000,
            entries: Vec::new(),
        };
        // Three copies of a 1000-byte file waste 2000, not 3000: one of them
        // is the file you were keeping anyway.
        let three = Group {
            entries: (0..3)
                .map(|i| FileEntry {
                    name: format!("{}.bin", i),
                    size: 1000,
                    ..Default::default()
                })
                .collect(),
            ..g
        };
        assert_eq!(three.reclaimable(), 2000);
    }

    #[test]
    fn the_biggest_waste_is_reported_first() {
        let base = tree(
            "fx_dupes_order",
            &[
                ("small1.bin", b"aa"),
                ("small2.bin", b"aa"),
                ("big1.bin", b"bbbbbbbbbbbbbbbbbbbb"),
                ("big2.bin", b"bbbbbbbbbbbbbbbbbbbb"),
            ],
        );
        let groups = scan(&base);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0],
            vec!["big1.bin".to_string(), "big2.bin".to_string()],
            "a scan that is still running should already show what matters"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_last_batch_is_always_marked_done() {
        let mut dones = 0;
        run(
            r"C:\definitely\not\here".into(),
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
    fn cancelling_stops_the_scan() {
        let base = tree(
            "fx_dupes_cancel",
            &[("a.txt", b"same"), ("b.txt", b"same")],
        );
        let mut groups = 0;
        run(
            base.to_string_lossy().into_owned(),
            1,
            Arc::new(AtomicBool::new(true)),
            |b| groups += b.groups.len(),
        );
        assert_eq!(groups, 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn group_numbers_continue_across_batches() {
        let group = |names: &[&str]| Group {
            size: 10,
            entries: names
                .iter()
                .map(|n| FileEntry {
                    name: n.to_string(),
                    size: 10,
                    ..Default::default()
                })
                .collect(),
        };

        let (entries, first) = rows(vec![group(&["a", "b"]), group(&["c", "d"])], 0);
        assert_eq!(entries.len(), 4, "every copy is a row, not every group");
        assert_eq!(
            first,
            vec![
                ("a".to_string(), 0),
                ("b".to_string(), 0),
                ("c".to_string(), 1),
                ("d".to_string(), 1),
            ]
        );

        // The next batch picks up where that one stopped. Restarting at zero
        // would give the bar the same colour either side of the seam.
        let (_, second) = rows(vec![group(&["e", "f"])], 2);
        assert_eq!(second, vec![("e".to_string(), 2), ("f".to_string(), 2)]);
    }

    #[test]
    fn a_clean_tree_says_so_in_words() {
        assert_eq!(summary(0, 0, 0, false), "No duplicate files found");
        assert_eq!(
            summary(0, 0, 0, true),
            "No duplicate files found (stopped early: too many files)",
            "and does not claim to have looked everywhere when it did not"
        );
    }

    #[test]
    fn the_summary_counts_copies_rather_than_files() {
        // Two files in one group is one redundant copy, not two.
        assert_eq!(
            summary(1, 2, 1024, false),
            "1 group of duplicates, 1 redundant copy, 1 KB reclaimable"
        );
        assert_eq!(
            summary(2, 5, 2048, false),
            "2 groups of duplicates, 3 redundant copies, 2 KB reclaimable"
        );
    }
}
