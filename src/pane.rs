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
            generation: 0,
            pending_select: None,
            history: Vec::new(),
            history_pos: 0,
        }
    }

    pub fn label(&self) -> String {
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
            .map(|e| fs::path_join(&dir, &e.name))
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

    /// Close a tab. The last tab is never destroyed — it is reset to the
    /// fallback path instead, so the pane always has something to show.
    /// (The old `close_tab` silently did nothing in this case.)
    pub fn close_tab(&mut self, index: usize, fallback: &str) -> Option<LoadRequest> {
        if index >= self.tabs.len() {
            return None;
        }
        if self.tabs.len() == 1 {
            let already_there = paths_equal(&self.tabs[0].path, fallback);
            if already_there {
                return None;
            }
            return Some(self.navigate(fallback));
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

    pub fn close_active_tab(&mut self, fallback: &str) -> Option<LoadRequest> {
        self.close_tab(self.active_tab_index, fallback)
    }

    /// Select `name` once the next load lands. Used after a rename or a new
            /// folder so the thing just created is the thing now highlighted.
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
    fn test_close_last_tab_resets_it_rather_than_no_op() {
        // Regression: the old close_tab on the final tab did nothing at all.
        let mut pane = Pane::new();
        let req = pane.navigate("C:\\Users\\Foo");
        load(&mut pane, &req, &[("a", false)]);
        let reset = pane.close_tab(0, "C:\\");
        assert!(reset.is_some(), "closing the last tab should navigate home");
        assert_eq!(reset.unwrap().path, "C:\\");
        assert_eq!(pane.tabs.len(), 1);
    }

    #[test]
    fn test_close_tab_keeps_active_index_on_the_same_tab() {
        let mut pane = Pane::new();
        pane.new_tab("C:\\b");
        pane.new_tab("C:\\c");
        pane.switch_tab(2);
        pane.close_tab(0, "C:\\");
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
