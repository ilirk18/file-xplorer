// The sidebar's folder tree.
//
// A flat list of visible rows derived from a set of expanded paths, rather
// than a linked structure of nodes: the sidebar draws rows, the layout assigns
// rectangles to rows, and hit-testing returns a row index. Keeping the model
// flat means all three agree about what row 7 is without anyone walking a tree.
//
// Children are read on expand and cached; collapsing drops the cache, so
// re-expanding a folder is also how you refresh it.
//
// ponytail: the read is synchronous. A local folder lists in under a
// millisecond, but expanding a node on a sleeping network share will stall the
// window. If that ever bites, route it through the same posted-message worker
// the panes use rather than adding a thread here.

use std::collections::{HashMap, HashSet};

use crate::fs;

/// One visible row of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub path: String,
    pub label: String,
    /// 0 for a drive, 1 for its children, and so on.
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Default)]
pub struct Tree {
    /// Lowercased paths that are open. Lowercased because Windows paths are
    /// case-insensitive and the same folder must not expand twice.
    expanded: HashSet<String>,
    /// Subfolders of each expanded path, in display order.
    children: HashMap<String, Vec<(String, String)>>,
}

/// Beyond this the sidebar is a column of indentation, and a symlink loop we
/// failed to notice would walk forever.
const MAX_DEPTH: usize = 16;

impl Tree {
    pub fn is_expanded(&self, path: &str) -> bool {
        self.expanded.contains(&path.to_lowercase())
    }

    /// Open or close `path`. Opening reads its subfolders; closing forgets
    /// them, which doubles as the refresh gesture.
    pub fn toggle(&mut self, path: &str) {
        let key = path.to_lowercase();
        if self.expanded.remove(&key) {
            self.children.remove(&key);
            return;
        }
        self.expanded.insert(key.clone());
        self.children.insert(key, read_subfolders(path));
    }

    /// Open every folder on the way down to `path`, so it becomes a visible
    /// row. Returns the path as the tree spells it, for the caller to scroll to.
    pub fn reveal(&mut self, path: &str) -> String {
        let trimmed = path.trim_end_matches('\\');
        let parts: Vec<&str> = trimmed.split('\\').collect();
        if parts.is_empty() {
            return String::new();
        }
        // Drives are roots and are spelled "C:\\"; everything below joins on.
        let mut so_far = format!("{}\\", parts[0]);
        for part in parts.iter().skip(1).take(MAX_DEPTH) {
            if !self.is_expanded(&so_far) {
                self.toggle(&so_far);
            }
            let next = fs::path_join(&so_far, part);
            // Listing skips hidden and reparse children on purpose — otherwise a
            // junction loops the tree. Reveal still has to walk through them
            // (AppData is hidden; Temp often sits behind it), so inject the
            // next hop when the listing left it out.
            self.ensure_child(&so_far, part, &next);
            so_far = next;
        }
        so_far
    }

    /// Make sure `child_path` appears under `parent` in the cached children.
    fn ensure_child(&mut self, parent: &str, label: &str, child_path: &str) {
        let key = parent.to_lowercase();
        let kids = self.children.entry(key).or_default();
        if kids
            .iter()
            .any(|(_, p)| p.eq_ignore_ascii_case(child_path))
        {
            return;
        }
        kids.push((label.to_string(), child_path.to_string()));
        kids.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    }

    /// Every row on screen, depth-first. `roots` are the top-level paths,
    /// normally the drive roots.
    pub fn rows(&self, roots: &[String]) -> Vec<TreeRow> {
        let mut out = Vec::new();
        // Reversed, because the stack pops from the back and the first root
        // has to come out first.
        let mut stack: Vec<(String, usize)> =
            roots.iter().rev().map(|r| (r.clone(), 0)).collect();

        while let Some((path, depth)) = stack.pop() {
            let expanded = self.is_expanded(&path);
            out.push(TreeRow {
                label: label_for(&path, depth),
                path: path.clone(),
                depth,
                expanded,
            });
            if !expanded || depth + 1 > MAX_DEPTH {
                continue;
            }
            if let Some(kids) = self.children.get(&path.to_lowercase()) {
                for (_, child_path) in kids.iter().rev() {
                    stack.push((child_path.clone(), depth + 1));
                }
            }
        }
        out
    }
}

/// A drive shows as "C:\"; anything deeper shows its own name.
fn label_for(path: &str, depth: usize) -> String {
    if depth == 0 {
        path.to_string()
    } else {
        fs::path_leaf(path)
    }
}

/// Subfolders only, and never a reparse point: following a junction here is
/// how a tree ends up showing the same folder under two names, or looping.
fn read_subfolders(path: &str) -> Vec<(String, String)> {
    let Ok(entries) = fs::list_dir(path) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = entries
        .into_iter()
        .filter(|e| e.is_dir && !e.is_reparse && !e.is_hidden)
        .map(|e| {
            let child = fs::path_join(path, &e.name);
            (e.name, child)
        })
        .collect();
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<String> {
        vec!["C:\\".to_string(), "D:\\".to_string()]
    }

    #[test]
    fn a_fresh_tree_shows_only_its_roots() {
        let t = Tree::default();
        let rows = t.rows(&roots());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "C:\\");
        assert_eq!(rows[0].depth, 0);
        assert!(!rows[0].expanded);
    }

    #[test]
    fn expanding_is_case_insensitive() {
        // Clicking "C:\" and then arriving from a path spelled "c:\" must not
        // open the same drive twice.
        let mut t = Tree::default();
        t.toggle("C:\\");
        assert!(t.is_expanded("c:\\"));
        t.toggle("c:\\");
        assert!(!t.is_expanded("C:\\"));
    }

    #[test]
    fn expanding_a_real_folder_lists_its_subfolders_in_order() {
        let base = std::env::temp_dir().join("fx_tree_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("beta")).unwrap();
        std::fs::create_dir_all(base.join("Alpha")).unwrap();
        std::fs::write(base.join("file.txt"), b"").unwrap();

        let root = base.to_string_lossy().into_owned();
        let mut t = Tree::default();
        t.toggle(&root);
        let rows = t.rows(&[root.clone()]);
        let names: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(names, vec![root.as_str(), "Alpha", "beta"]);
        assert_eq!(rows[1].depth, 1);

        // Collapsing forgets the children, so re-expanding re-reads.
        t.toggle(&root);
        assert_eq!(t.rows(&[root.clone()]).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_collapsed_parent_hides_its_whole_subtree() {
        let base = std::env::temp_dir().join("fx_tree_nested");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("one").join("two")).unwrap();
        let root = base.to_string_lossy().into_owned();
        let one = fs::path_join(&root, "one");

        let mut t = Tree::default();
        t.toggle(&root);
        t.toggle(&one);
        assert_eq!(t.rows(&[root.clone()]).len(), 3);
        t.toggle(&root);
        assert_eq!(t.rows(&[root.clone()]).len(), 1, "the subtree goes with it");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn revealing_a_path_opens_every_folder_above_it() {
        let base = std::env::temp_dir().join("fx_tree_reveal");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("one").join("two")).unwrap();
        let deep = base.join("one").join("two").to_string_lossy().into_owned();

        let mut t = Tree::default();
        let landed = t.reveal(&deep);
        assert_eq!(landed.to_lowercase(), deep.to_lowercase());
        // Every ancestor is open, so the row is actually on screen...
        let rows = t.rows(&["C:\\".to_string()]);
        assert!(
            rows.iter().any(|r| r.path.eq_ignore_ascii_case(&deep)),
            "the revealed folder must be one of the visible rows"
        );
        // ...and the folder itself is left closed, which is where you want to
        // arrive: looking at it, not inside it.
        assert!(!t.is_expanded(&deep));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn roots_keep_their_order() {
        let mut t = Tree::default();
        t.toggle("C:\\nonexistent-for-sure");
        let rows = t.rows(&roots());
        assert_eq!(rows[0].path, "C:\\");
        assert_eq!(rows[1].path, "D:\\");
    }
}
