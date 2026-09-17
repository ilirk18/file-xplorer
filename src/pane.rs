// One pane: a stack of tabs, each with its own listing, history, and scroll.
//
// Directory reads happen on a worker thread, so a pane never holds a listing
// directly. It issues a `LoadRequest` carrying a generation number, and only
// applies the result if that generation is still current. Without that, a slow
// network folder would overwrite whatever the user navigated to in the meantime.

use crate::file_list::FileList;
use crate::fs::{self, FileEntry};

/// Whether a load replaces the view or updates it in place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadKind {
    /// Going somewhere new: reset scroll and selection.
    Navigate,
    /// Same directory, fresh contents: keep scroll and selection.
    Refresh,
}

/// A directory read for the worker thread to perform.
#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub tab_id: u64,
    pub generation: u64,
    pub path: String,
    pub kind: LoadKind,
    /// Entry to select once loaded, e.g. the folder we just stepped out of.
    pub select_after: Option<String>,
}

pub struct Tab {
    pub id: u64,
    pub path: String,
    pub file_list: FileList,
    /// True between issuing a LoadRequest and applying its result.
    pub loading: bool,
    /// Set when the last load failed, shown in place of the listing.
    pub error: Option<String>,
    /// Some(query) while this tab shows search results rather than a listing.
    /// `path` stays the search root, so opening a result still works through
    /// the ordinary path_join(current_path, name).
    pub search_query: Option<String>,
    /// Running totals for a duplicate scan, which streams its findings in and
    /// so cannot state its own result until the last batch. Counted on the tab
    /// rather than in the app, because two panes can be scanning at once.
    pub dup_groups: u32,
    pub dup_files: usize,
    pub dup_bytes: u64,

    generation: u64,
    pending_select: Option<String>,
    history: Vec<String>,
    history_pos: usize,
}

impl Tab {
    fn new(id: u64) -> Self {
        Self {
            id,
            path: String::new(),
            file_list: FileList::default(),
            loading: false,
            error: None,
            search_query: None,
            dup_groups: 0,
            dup_files: 0,
            dup_bytes: 0,
            generation: 0,
            pending_select: None,
            history: Vec::new(),
            history_pos: 0,
        }
    }

    pub fn label(&self) -> String {
        if let Some(q) = &self.search_query {
            return q.clone();
        }
        if self.path.is_empty() {
            "New Tab".to_string()
        } else {
            fs::path_leaf(&self.path)
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.history_pos > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_pos + 1 < self.history.len()
    }
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active_tab_index: usize,
    /// Rows are this tall; kept on the pane so DPI changes apply to every tab.
    pub row_height: u32,
    pub show_hidden: bool,
    next_tab_id: u64,
}

impl Default for Pane {
    fn default() -> Self {
        Self::new()
    }
}

impl Pane {
    pub fn new() -> Self {
        let mut p = Self {
            tabs: Vec::new(),
            active_tab_index: 0,
            row_height: 24,
            show_hidden: false,
            next_tab_id: 1,
        };
        p.tabs.push(Tab::new(0));
        p.next_tab_id = 1;
        p
    }

    // -- active tab accessors ---------------------------------------------

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active_tab_index.min(self.tabs.len() - 1)]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        let i = self.active_tab_index.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    pub fn list(&self) -> &FileList {
        &self.active().file_list
    }

    pub fn list_mut(&mut self) -> &mut FileList {
        &mut self.active_mut().file_list
    }

    pub fn current_path(&self) -> &str {
        &self.active().path
    }

    /// Absolute paths of the current selection.
    pub fn selected_paths(&self) -> Vec<String> {
        let dir = self.current_path().to_string();
        self.list()
            .selected_entries()
            .iter()
            .map(|e| fs::child_path(&dir, e))
            .collect()
    }

    // -- navigation --------------------------------------------------------

    /// Begin loading `path` into the active tab, pushing onto the history.
    pub fn navigate(&mut self, path: &str) -> LoadRequest {
        let path = path.to_string();
        {
            let tab = self.active_mut();
            // Drop any forward history, the way a browser does.
            tab.history.truncate(tab.history_pos + 1);
            if tab.history.last().map(|s| s.as_str()) != Some(path.as_str()) {
                if tab.history.is_empty() {
                    tab.history.push(path.clone());
                    tab.history_pos = 0;
                } else {
                    tab.history.push(path.clone());
                    tab.history_pos = tab.history.len() - 1;
                }
            }
        }
        self.begin_load(path, LoadKind::Navigate, None)
    }

    /// Go to the parent directory, selecting the folder we came from.
    pub fn navigate_up(&mut self) -> Option<LoadRequest> {
        let current = self.current_path().to_string();
        let parent = fs::path_parent(&current)?;
        let leaf = fs::path_leaf(&current);
        let mut req = self.navigate(&parent);
        req.select_after = Some(leaf.clone());
        self.active_mut().pending_select = Some(leaf);
        Some(req)
    }

    pub fn go_back(&mut self) -> Option<LoadRequest> {
        let path = {
            let tab = self.active_mut();
            if !tab.can_go_back() {
                return None;
            }
            tab.history_pos -= 1;
            tab.history[tab.history_pos].clone()
        };
        Some(self.begin_load(path, LoadKind::Navigate, None))
    }

    pub fn go_forward(&mut self) -> Option<LoadRequest> {
        let path = {
            let tab = self.active_mut();
            if !tab.can_go_forward() {
                return None;
            }
            tab.history_pos += 1;
            tab.history[tab.history_pos].clone()
        };
        Some(self.begin_load(path, LoadKind::Navigate, None))
    }

    /// Re-read the current directory in place.
    pub fn refresh(&mut self) -> Option<LoadRequest> {
        let path = self.current_path().to_string();
        if path.is_empty() {
            return None;
        }
        Some(self.begin_load(path, LoadKind::Refresh, None))
    }

    fn begin_load(
        &mut self,
        path: String,
        kind: LoadKind,
        select_after: Option<String>,
    ) -> LoadRequest {
        let show_hidden = self.show_hidden;
        let row_height = self.row_height;
        let tab = self.active_mut();
        tab.generation += 1;
        tab.loading = true;
        tab.error = None;
        tab.search_query = None;
        tab.file_list.set_show_hidden(show_hidden);
        tab.file_list.row_height = row_height;
        if let Some(sel) = select_after.clone() {
            tab.pending_select = Some(sel);
        }
        LoadRequest {
            tab_id: tab.id,
            generation: tab.generation,
            path,
            kind,
            select_after,
        }
    }

    /// Apply a finished directory read. Stale results (a newer load has since
    /// been issued, or the tab was closed) are dropped.
    ///
    /// Returns true if anything changed and the window needs repainting.
    pub fn finish_load(
        &mut self,
        tab_id: u64,
        generation: u64,
        path: &str,
        kind: LoadKind,
        result: Result<Vec<FileEntry>, String>,
        list_height: u32,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if tab.generation != generation {
            return false;
        }
        tab.loading = false;
        match result {
            Ok(entries) => {
                // A Refresh keeps scroll and selection. So does a Navigate that
                // happens to land on the directory already shown, which is what
                // clicking the current breadcrumb does.
                let in_place = kind == LoadKind::Refresh || paths_equal(&tab.path, path);
                tab.path = path.to_string();
                tab.error = None;
                if in_place {
                    tab.file_list.refresh_entries(entries);
                } else {
                    tab.file_list.set_entries(entries);
                }
                if let Some(name) = tab.pending_select.take() {
                    tab.file_list.select_by_name(&name, list_height);
                }
            }
            Err(message) => {
                // Keep showing whatever was there; report the failure instead of
                // silently doing nothing, which is what every `let _ =` did before.
                tab.error = Some(message);
                tab.pending_select = None;
            }
        }
        true
    }

    // -- tabs --------------------------------------------------------------

    /// Open a new tab at `path` and make it active. Returns its load request.
    pub fn new_tab(&mut self, path: &str) -> LoadRequest {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Tab::new(id);
        tab.file_list.row_height = self.row_height;
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
        self.navigate(path)
    }

    /// Take the pane's only tab, leaving it holding `fallback` instead.
    ///
    /// A swap, not a removal: a pane is never without a tab, because `active`
    /// would have nothing to return. For the caller that is about to close the
    /// pane \u2014 what stays behind is what the slot holds if it is ever used
    /// again.
    pub fn take_only_tab(&mut self, fallback: &str) -> Option<Tab> {
        if self.tabs.len() != 1 {
            return None;
        }
        // The replacement first, so there is never a moment with no tabs.
        let _ = self.new_tab(fallback);
        let tab = self.tabs.remove(0);
        self.active_tab_index = 0;
        Some(tab)
    }

    /// Rebuild the tab strip from a saved session.
    ///
    /// Only the active tab is loaded. The others get their path and history
    /// but no listing, so `switch_tab` reads them the first time they are
    /// shown — restoring twenty tabs must not mean twenty directory reads at
    /// startup.
    pub fn restore(&mut self, paths: &[String], active: usize) -> Option<LoadRequest> {
        if paths.is_empty() {
            return None;
        }
        self.tabs.clear();
        for path in paths {
            let id = self.next_tab_id;
            self.next_tab_id += 1;
            let mut tab = Tab::new(id);
            tab.file_list.row_height = self.row_height;
            tab.file_list.set_show_hidden(self.show_hidden);
            tab.path = path.clone();
            tab.history.push(path.clone());
            tab.history_pos = 0;
            self.tabs.push(tab);
        }
        self.active_tab_index = active.min(self.tabs.len() - 1);
        let path = self.current_path().to_string();
        Some(self.begin_load(path, LoadKind::Navigate, None))
    }

    /// Detach a tab so another pane can adopt it. The last tab is never
    /// detached: a pane with no tabs has nothing to show and nowhere to
    /// navigate from.
    /// Move a tab within this pane, for dragging one past another.
    ///
    /// The active tab stays active wherever it lands, which is why this works
    /// by id rather than by index: `remove` then `insert` shifts everything in
    /// between, and half of those shifts are in the opposite direction.
    pub fn reorder_tab(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return false;
        }
        let active_id = self.active_tab_id();
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(i) = self.tabs.iter().position(|t| t.id == active_id) {
            self.active_tab_index = i;
        }
        true
    }

    pub fn take_tab(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() || self.tabs.len() == 1 {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        } else if index < self.active_tab_index {
            self.active_tab_index -= 1;
        }
        Some(tab)
    }

    /// Adopt a tab from another pane and make it active. Its id is reissued,
    /// because ids only have to be unique within a pane and the one it brought
    /// may already be taken here.
    pub fn adopt_tab(&mut self, mut tab: Tab) -> Option<LoadRequest> {
        tab.id = self.next_tab_id;
        self.next_tab_id += 1;
        tab.file_list.row_height = self.row_height;
        tab.file_list.set_show_hidden(self.show_hidden);
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
        // Already loaded: the listing came with it and needs no re-read.
        if !self.active().file_list.entries.is_empty() {
            return None;
        }
        let path = self.current_path().to_string();
        if path.is_empty() {
            return None;
        }
        Some(self.begin_load(path, LoadKind::Refresh, None))
    }

    /// The paths of every open tab, for saving the session.
    /// The selection as shell id lists, for anything that has to ask the shell
    /// about these items rather than about their paths.
    pub fn selected_pidls(&self) -> Vec<crate::pidl::Pidl> {
        let dir = self.current_path();
        self.list()
            .selected_entries()
            .iter()
            .filter_map(|e| fs::entry_pidl(dir, e))
            .collect()
    }

    pub fn tab_paths(&self) -> Vec<String> {
        self.tabs
            .iter()
            .map(|t| t.path.clone())
            .filter(|p| !p.is_empty())
            .collect()
    }

    /// Duplicate the active tab, which is what Ctrl+T should do from a folder.
    pub fn duplicate_tab(&mut self) -> LoadRequest {
        let path = self.current_path().to_string();
        self.new_tab(&path)
    }

    pub fn switch_tab(&mut self, index: usize) -> Option<LoadRequest> {
        if index >= self.tabs.len() || index == self.active_tab_index {
            return None;
        }
        self.active_tab_index = index;
        // Each tab keeps its own listing, so switching is instant. Only reload
        // if the tab has never been populated.
        if self.tabs[index].file_list.entries.is_empty() && !self.tabs[index].path.is_empty() {
            let path = self.tabs[index].path.clone();
            return Some(self.begin_load(path, LoadKind::Refresh, None));
        }
        None
    }

    pub fn next_tab(&mut self, delta: i32) -> Option<LoadRequest> {
        if self.tabs.len() < 2 {
            return None;
        }
        let n = self.tabs.len() as i32;
        let i = (self.active_tab_index as i32 + delta).rem_euclid(n) as usize;
        self.switch_tab(i)
    }

    /// Close a tab, of a pane that has more than one.
    ///
    /// A pane's last tab is not this function's to close: the pane goes, or
    /// with no other pane the window does, and neither is a pane's decision.
    /// See `input::close_tab`.
    pub fn close_tab(&mut self, index: usize) -> Option<LoadRequest> {
        if index >= self.tabs.len() || self.tabs.len() == 1 {
            return None;
        }
        self.tabs.remove(index);
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        } else if index < self.active_tab_index {
            self.active_tab_index -= 1;
        }

        let active = self.active_tab_index;
        if self.tabs[active].file_list.entries.is_empty() && !self.tabs[active].path.is_empty() {
            let path = self.tabs[active].path.clone();
            return Some(self.begin_load(path, LoadKind::Refresh, None));
        }
        None
    }

    /// Select `name` once the next load lands. Used after a rename or a new
            /// folder so the thing just created is the thing now highlighted.
    /// Switch the active tab into search mode. Shares the load generation, so
    /// navigating away while a search runs discards its remaining results.
    pub fn begin_search(&mut self, query: &str) -> (u64, u64, String) {
        let root = self.current_path().to_string();
        let tab = self.active_mut();
        tab.generation += 1;
        tab.loading = true;
        tab.error = None;
        tab.search_query = Some(query.to_string());
        // A second scan in the same tab starts its findings from zero.
        tab.dup_groups = 0;
        tab.dup_files = 0;
        tab.dup_bytes = 0;
        tab.file_list.set_entries(Vec::new());
        (tab.id, tab.generation, root)
    }

    /// Apply one batch of search results. Stale batches are dropped, exactly
    /// like stale directory reads.
    pub fn finish_search_batch(
        &mut self,
        tab_id: u64,
        generation: u64,
        entries: Vec<FileEntry>,
        done: bool,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if tab.generation != generation || tab.search_query.is_none() {
            return false;
        }
        tab.file_list.append_entries(entries);
        if done {
            tab.loading = false;
        }
        true
    }

    pub fn is_searching(&self) -> bool {
        self.active().search_query.is_some()
    }

    pub fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn active_tab_id(&self) -> u64 {
        self.active().id
    }

    pub fn select_after_next_load(&mut self, name: &str) {
        self.active_mut().pending_select = Some(name.to_string());
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        for tab in &mut self.tabs {
            tab.file_list.set_show_hidden(show);
        }
    }

    pub fn set_row_height(&mut self, h: u32) {
        self.row_height = h;
        for tab in &mut self.tabs {
            tab.file_list.row_height = h;
        }
    }

    /// Entries per line, from the layout. Every tab in the pane agrees, so
    /// switching tabs does not switch view.
    pub fn set_columns(&mut self, cols: u32) {
        for tab in &mut self.tabs {
            tab.file_list.columns = cols.max(1);
        }
    }
}

/// Windows paths are case-insensitive, and a trailing slash is not a difference.
pub fn paths_equal(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let t = s.trim_end_matches('\\');
        if t.is_empty() {
            s.to_lowercase()
        } else {
            t.to_lowercase()
        }
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::FileEntry;

    #[test]
    fn restoring_one_path_collapses_a_pane_to_one_tab() {
        // What a split relies on: the slot it is handed may still hold the
        // tabs it had before it was closed, and a new pane shows one folder.
        let mut p = Pane::new();
        p.new_tab(r"C:");
        p.new_tab(r"C:");
        assert_eq!(p.tabs.len(), 3);
        p.restore(&[r"C:\here".to_string()], 0);
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.active_tab_index, 0);
        assert_eq!(p.current_path(), r"C:\here");
    }

    #[test]
    fn reorder_keeps_the_active_tab_active() {
        let mut p = Pane::new();
        p.new_tab(r"C:");
        p.new_tab(r"C:\c");
        // new_tab activates what it opened, so the third tab is active.
        let ids: Vec<u64> = p.tabs.iter().map(|t| t.id).collect();
        let active = p.active_tab_id();
        assert!(p.reorder_tab(2, 0));
        assert_eq!(p.active_tab_id(), active);
        assert_eq!(p.active_tab_index, 0);
        assert_eq!(
            p.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![ids[2], ids[0], ids[1]]
        );
    }

    #[test]
    fn reorder_shifting_past_the_active_tab_tracks_it() {
        let mut p = Pane::new();
        p.new_tab(r"C:");
        p.new_tab(r"C:\c");
        p.switch_tab(0);
        let active = p.active_tab_id();
        assert!(p.reorder_tab(2, 0));
        assert_eq!(p.active_tab_id(), active);
        assert_eq!(p.active_tab_index, 1, "the active tab was pushed right");
    }

    #[test]
    fn reorder_rejects_nonsense() {
        let mut p = Pane::new();
        assert!(!p.reorder_tab(0, 0), "no-op");
        assert!(!p.reorder_tab(0, 9), "out of range");
    }

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            size: 0,
            modified: 0,
            is_dir,
            is_reparse: false,
            is_hidden: false,
            dir_size_known: false,
            extension: None,
            target: None,
            pidl: None,
        }
    }

    fn load(pane: &mut Pane, req: &LoadRequest, names: &[(&str, bool)]) {
        pane.finish_load(
            req.tab_id,
            req.generation,
            &req.path,
            req.kind,
            Ok(names.iter().map(|(n, d)| entry(n, *d)).collect()),
            240,
        );
    }

    #[test]
    fn test_paths_equal_is_case_and_slash_insensitive() {
        assert!(paths_equal("C:\\Users", "c:\\users"));
        assert!(paths_equal("C:\\Users\\", "C:\\Users"));
        assert!(!paths_equal("C:\\Users", "C:\\Windows"));
    }

    #[test]
    fn test_new_tab_is_actually_reachable() {
        // Regression: the old Pane could never grow past one tab.
        let mut pane = Pane::new();
        assert_eq!(pane.tabs.len(), 1);
        pane.new_tab("C:\\Users");
        assert_eq!(pane.tabs.len(), 2);
        assert_eq!(pane.active_tab_index, 1);
    }

    #[test]
    fn test_a_panes_last_tab_stays_put() {
        // The pane keeps showing what it was showing. Closing it is the
        // caller's call: the pane goes, or the window does.
        let mut pane = Pane::new();
        let req = pane.navigate("C:\\Users\\Foo");
        load(&mut pane, &req, &[("a", false)]);
        assert!(pane.close_tab(0).is_none());
        assert_eq!(pane.tabs.len(), 1);
        assert_eq!(pane.current_path(), "C:\\Users\\Foo");
    }

    #[test]
    fn test_close_tab_keeps_active_index_on_the_same_tab() {
        let mut pane = Pane::new();
        pane.new_tab("C:\\b");
        pane.new_tab("C:\\c");
        pane.switch_tab(2);
        pane.close_tab(0);
        assert_eq!(pane.tabs.len(), 2);
        assert_eq!(pane.active_tab_index, 1, "still on the tab that was active");
    }

    #[test]
    fn test_tabs_keep_independent_listings() {
        let mut pane = Pane::new();
        let r0 = pane.navigate("C:\\one");
        load(&mut pane, &r0, &[("a.txt", false), ("b.txt", false)]);

        let r1 = pane.new_tab("C:\\two");
        load(&mut pane, &r1, &[("z.txt", false)]);
        assert_eq!(pane.list().entries.len(), 1);

        // Switching back must not need a reload.
        assert!(pane.switch_tab(0).is_none());
        assert_eq!(pane.list().entries.len(), 2);
    }

    #[test]
    fn a_tab_can_move_between_panes_with_its_listing() {
        let mut a = Pane::new();
        let r0 = a.navigate("C:\\one");
        load(&mut a, &r0, &[("x.txt", false)]);
        let r1 = a.new_tab("C:\\two");
        load(&mut a, &r1, &[("y.txt", false)]);

        let mut b = Pane::new();
        let moved = a.take_tab(1).expect("two tabs, so one can leave");
        assert_eq!(a.tabs.len(), 1);
        // The listing travels with it, so there is nothing to reload.
        assert!(b.adopt_tab(moved).is_none());
        assert_eq!(b.current_path(), "C:\\two");
        assert_eq!(b.list().entries[0].name, "y.txt");
    }

    #[test]
    fn the_last_tab_never_leaves_its_pane() {
        // A pane with no tabs has nothing to show and nowhere to navigate from.
        let mut p = Pane::new();
        assert!(p.take_tab(0).is_none());
    }

    #[test]
    fn an_adopted_tab_gets_an_id_that_is_free_here() {
        let mut a = Pane::new();
        a.new_tab("C:\\two");
        let mut b = Pane::new();
        b.new_tab("C:\\other");
        let moved = a.take_tab(1).unwrap();
        let old_id = moved.id;
        b.adopt_tab(moved);
        assert_ne!(b.active().id, old_id, "reissued, or a load could land twice");
        let ids: Vec<u64> = b.tabs.iter().map(|t| t.id).collect();
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(ids.len(), uniq.len());
    }

    #[test]
    fn a_restored_session_loads_only_the_active_tab() {
        let mut pane = Pane::new();
        let req = pane
            .restore(&["C:\\a".into(), "C:\\b".into(), "C:\\c".into()], 1)
            .expect("three tabs is a session");
        assert_eq!(pane.tabs.len(), 3);
        assert_eq!(req.path, "C:\\b", "the active tab is the one that loads");
        assert_eq!(pane.tab_paths(), vec!["C:\\a", "C:\\b", "C:\\c"]);
        // The rest keep their path but no listing, so they read on first show.
        assert!(pane.switch_tab(2).is_some());
    }

    #[test]
    fn a_restored_active_index_past_the_end_lands_on_the_last_tab() {
        let mut pane = Pane::new();
        let req = pane.restore(&["C:\\a".into()], 7).unwrap();
        assert_eq!(req.path, "C:\\a");
        assert_eq!(pane.active_tab_index, 0);
    }

    #[test]
    fn an_empty_session_leaves_the_pane_alone() {
        let mut pane = Pane::new();
        assert!(pane.restore(&[], 0).is_none());
        assert_eq!(pane.tabs.len(), 1);
    }

    #[test]
    fn test_stale_load_is_discarded() {
        let mut pane = Pane::new();
        let slow = pane.navigate("C:\\slow");
        let fast = pane.navigate("C:\\fast");
        load(&mut pane, &fast, &[("current.txt", false)]);

        // The slow read finally lands, but it is a generation behind.
        let applied = pane.finish_load(
            slow.tab_id,
            slow.generation,
            &slow.path,
            slow.kind,
            Ok(vec![entry("stale.txt", false)]),
            240,
        );
        assert!(!applied, "stale result must be dropped");
        assert_eq!(pane.current_path(), "C:\\fast");
        assert_eq!(pane.list().entries[0].name, "current.txt");
    }

    #[test]
    fn test_navigate_up_selects_the_folder_left_behind() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\Users\\Foo");
        load(&mut pane, &r, &[("x", false)]);

        let up = pane.navigate_up().expect("has a parent");
        assert_eq!(up.path, "C:\\Users");
        pane.finish_load(
            up.tab_id,
            up.generation,
            &up.path,
            up.kind,
            Ok(vec![entry("Bar", true), entry("Foo", true)]),
            240,
        );
        let sel = pane.list().selected_entries();
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0].name, "Foo");
    }

    #[test]
    fn test_navigate_up_stops_at_root() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\");
        load(&mut pane, &r, &[]);
        assert!(pane.navigate_up().is_none());
    }

    #[test]
    fn test_history_back_and_forward() {
        let mut pane = Pane::new();
        for p in ["C:\\a", "C:\\b", "C:\\c"] {
            let r = pane.navigate(p);
            load(&mut pane, &r, &[]);
        }
        assert!(pane.active().can_go_back());
        assert!(!pane.active().can_go_forward());

        let back = pane.go_back().unwrap();
        assert_eq!(back.path, "C:\\b");
        load(&mut pane, &back, &[]);
        assert!(pane.active().can_go_forward());

        let fwd = pane.go_forward().unwrap();
        assert_eq!(fwd.path, "C:\\c");
    }

    #[test]
    fn test_navigating_truncates_forward_history() {
        let mut pane = Pane::new();
        for p in ["C:\\a", "C:\\b", "C:\\c"] {
            let r = pane.navigate(p);
            load(&mut pane, &r, &[]);
        }
        let back = pane.go_back().unwrap();
        load(&mut pane, &back, &[]);
        let r = pane.navigate("C:\\d");
        load(&mut pane, &r, &[]);
        assert!(!pane.active().can_go_forward(), "forward history is dropped");
    }

    #[test]
    fn test_refresh_keeps_selection() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\a");
        load(&mut pane, &r, &[("one.txt", false), ("two.txt", false)]);
        pane.list_mut()
            .select(1, crate::file_list::SelectMode::Replace);

        let rr = pane.refresh().unwrap();
        assert_eq!(rr.kind, LoadKind::Refresh);
        load(&mut pane, &rr, &[("one.txt", false), ("two.txt", false), ("new.txt", false)]);
        assert_eq!(pane.list().selected_entries()[0].name, "two.txt");
    }

    #[test]
    fn search_results_stream_in_and_stale_ones_are_dropped() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\a");
        load(&mut pane, &r, &[("x.txt", false)]);

        let (tab, gen, root) = pane.begin_search("rs");
        assert_eq!(root, "C:\\a");
        assert!(pane.is_searching());
        assert_eq!(pane.list().entries.len(), 0, "results start empty");

        assert!(pane.finish_search_batch(tab, gen, vec![entry("sub\\one.rs", false)], false));
        assert!(pane.finish_search_batch(tab, gen, vec![entry("two.rs", false)], true));
        assert_eq!(pane.list().entries.len(), 2);
        assert!(!pane.active().loading);

        // A batch from a superseded search must not land.
        assert!(!pane.finish_search_batch(tab, gen - 1, vec![entry("old.rs", false)], true));
    }

    #[test]
    fn navigating_leaves_search_mode() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\a");
        load(&mut pane, &r, &[]);
        let (tab, gen, _) = pane.begin_search("q");
        pane.finish_search_batch(tab, gen, vec![entry("hit.rs", false)], true);
        assert!(pane.is_searching());

        let r2 = pane.navigate("C:\\b");
        load(&mut pane, &r2, &[("plain.txt", false)]);
        assert!(!pane.is_searching());
        assert_eq!(pane.active().label(), "b");
    }

    #[test]
    fn a_searching_tab_is_labelled_as_such() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\a");
        load(&mut pane, &r, &[]);
        // What the real caller passes: a label, already saying what kind of
        // search this is. Prefixing it again gave "Search: Search: *.rs".
        pane.begin_search(&crate::search::Query::by_name("*.rs").label());
        assert_eq!(pane.active().label(), "Search: *.rs");

        pane.begin_search("Duplicates in a");
        assert_eq!(
            pane.active().label(),
            "Duplicates in a",
            "a scan that is not a name search says so"
        );
    }

    #[test]
    fn test_load_error_is_reported_not_swallowed() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\denied");
        pane.finish_load(
            r.tab_id,
            r.generation,
            &r.path,
            r.kind,
            Err("Access is denied.".into()),
            240,
        );
        assert_eq!(pane.active().error.as_deref(), Some("Access is denied."));
        assert!(!pane.active().loading);
    }

    #[test]
    fn test_selected_paths_are_absolute() {
        let mut pane = Pane::new();
        let r = pane.navigate("C:\\a");
        load(&mut pane, &r, &[("one.txt", false), ("two.txt", false)]);
        pane.list_mut().select_all();
        let paths = pane.selected_paths();
        assert_eq!(paths, vec!["C:\\a\\one.txt", "C:\\a\\two.txt"]);
    }
}
