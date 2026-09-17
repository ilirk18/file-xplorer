// Virtual list: what is on screen, what is selected, and in what order.
//
// The list owns the selection because every file operation acts on it, and it
// owns the sort because the column headers drive both.

use crate::fs::FileEntry;
use std::collections::HashSet;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Date,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    pub fn flipped(self) -> Self {
        match self {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        }
    }
}

/// How a click modifies the selection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click: this row becomes the whole selection.
    Replace,
    /// Ctrl: toggle this row, keep the rest.
    Toggle,
    /// Shift: select from the anchor to here.
    Range,
}

pub struct FileList {
    pub entries: Vec<FileEntry>,
    pub scroll_offset: u32,
    pub row_height: u32,
    pub sort_key: SortKey,
    pub sort_order: SortOrder,
    pub show_hidden: bool,

    /// Every selected row index.
    selected: HashSet<u32>,
    /// The row the keyboard acts from, drawn with a focus outline. Not always
    /// selected: Ctrl+arrow moves the cursor without changing the selection.
    cursor: Option<u32>,
    /// Where a Shift-range selection measures from.
    anchor: Option<u32>,
}

impl Default for FileList {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            row_height: 24,
            sort_key: SortKey::Name,
            sort_order: SortOrder::Asc,
            show_hidden: false,
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
        }
    }
}

impl FileList {
    // -- geometry ----------------------------------------------------------

    /// Visible row range as (start, end_exclusive).
    pub fn visible_range(&self, client_height: u32) -> (u32, u32) {
        if self.entries.is_empty() {
            return (0, 0);
        }
        let start = self.scroll_offset;
        let end = (start + self.visible_row_count(client_height) + 1).min(self.entries.len() as u32);
        (start, end)
    }

    /// Whole rows that fit in `client_height`.
    pub fn visible_row_count(&self, client_height: u32) -> u32 {
        if self.row_height == 0 {
            return 0;
        }
        client_height / self.row_height
    }

    pub fn total_rows(&self) -> u32 {
        self.entries.len() as u32
    }

    pub fn max_scroll(&self, client_height: u32) -> u32 {
        let visible = self.visible_row_count(client_height);
        (self.entries.len() as u32).saturating_sub(visible)
    }

    pub fn scroll_to(&mut self, offset: u32, client_height: u32) {
        self.scroll_offset = offset.min(self.max_scroll(client_height));
    }

    pub fn scroll_by(&mut self, delta: i32, client_height: u32) {
        let max = self.max_scroll(client_height) as i32;
        self.scroll_offset = (self.scroll_offset as i32 + delta).clamp(0, max.max(0)) as u32;
    }

    /// Scroll the minimum distance needed to bring `index` on screen.
    ///
    /// The old Up handler re-centred the viewport on every keypress whenever the
    /// list was scrolled at all, which made Up and Down behave differently and
    /// jerked the list around. Minimal scrolling is what every list control does.
    pub fn ensure_visible(&mut self, index: u32, client_height: u32) {
        let visible = self.visible_row_count(client_height);
        if visible == 0 {
            return;
        }
        if index < self.scroll_offset {
            self.scroll_offset = index;
        } else if index >= self.scroll_offset + visible {
            self.scroll_offset = index.saturating_sub(visible).saturating_add(1);
        }
        self.scroll_offset = self.scroll_offset.min(self.max_scroll(client_height));
    }

    // -- selection ---------------------------------------------------------

    pub fn cursor(&self) -> Option<u32> {
        self.cursor
    }

    pub fn is_selected(&self, index: u32) -> bool {
        self.selected.contains(&index)
    }

    pub fn selection_count(&self) -> usize {
        self.selected.len()
    }

    /// Selected entries in list order.
    pub fn selected_entries(&self) -> Vec<&FileEntry> {
        let mut idx: Vec<u32> = self.selected.iter().copied().collect();
        idx.sort_unstable();
        idx.iter()
            .filter_map(|i| self.entries.get(*i as usize))
            .collect()
    }

    /// Total bytes across the selection, for the status bar.
    pub fn selected_size(&self) -> u64 {
        self.selected_entries()
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.size)
            .sum()
    }

    /// The entry the keyboard would act on: the cursor row.
    pub fn cursor_entry(&self) -> Option<&FileEntry> {
        self.cursor.and_then(|i| self.entries.get(i as usize))
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn select_all(&mut self) {
        self.selected = (0..self.entries.len() as u32).collect();
        if self.cursor.is_none() && !self.entries.is_empty() {
            self.cursor = Some(0);
            self.anchor = Some(0);
        }
    }

    /// Apply a click (or a Shift/Ctrl arrow key) at `index`.
    pub fn select(&mut self, index: u32, mode: SelectMode) {
        if index as usize >= self.entries.len() {
            return;
        }
        match mode {
            SelectMode::Replace => {
                self.selected.clear();
                self.selected.insert(index);
                self.anchor = Some(index);
            }
            SelectMode::Toggle => {
                if !self.selected.remove(&index) {
                    self.selected.insert(index);
                }
                self.anchor = Some(index);
            }
            SelectMode::Range => {
                let from = self.anchor.unwrap_or(index);
                self.selected.clear();
                let (lo, hi) = if from <= index { (from, index) } else { (index, from) };
                for i in lo..=hi {
                    self.selected.insert(i);
                }
                // The anchor deliberately stays put so successive Shift+clicks
                // grow and shrink the same range.
            }
        }
        self.cursor = Some(index);
    }

    /// Move the cursor by `delta` rows and apply `mode` at the destination.
    /// Returns the new cursor row so the caller can scroll it into view.
    pub fn move_cursor(&mut self, delta: i32, mode: SelectMode) -> Option<u32> {
        let len = self.entries.len() as u32;
        if len == 0 {
            return None;
        }
        // With no cursor yet, the first Down lands on row 0 rather than row 1.
        // The old code did `unwrap_or(0) + 1` and skipped the first entry.
        let next = match self.cursor {
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
            Some(cur) => (cur as i64 + delta as i64).clamp(0, (len - 1) as i64) as u32,
        };
        self.select(next, mode);
        Some(next)
    }

    /// Move the cursor without touching the selection (Ctrl+arrow).
    pub fn move_cursor_only(&mut self, delta: i32) -> Option<u32> {
        let len = self.entries.len() as u32;
        if len == 0 {
            return None;
        }
        let next = match self.cursor {
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
            Some(cur) => (cur as i64 + delta as i64).clamp(0, (len - 1) as i64) as u32,
        };
        self.cursor = Some(next);
        Some(next)
    }

    /// Jump the cursor to an absolute row (Home / End).
    pub fn cursor_to(&mut self, index: u32, mode: SelectMode) -> Option<u32> {
        if self.entries.is_empty() {
            return None;
        }
        let index = index.min(self.entries.len() as u32 - 1);
        self.select(index, mode);
        Some(index)
    }

    /// First entry whose name starts with `prefix`, searching from just after
    /// the cursor and wrapping. Drives type-to-select.
    pub fn find_prefix(&self, prefix: &str) -> Option<u32> {
        if prefix.is_empty() || self.entries.is_empty() {
            return None;
        }
        let lower = prefix.to_lowercase();
        let len = self.entries.len();
        let start = self.cursor.map(|c| c as usize).unwrap_or(0);
        for step in 0..len {
            let i = (start + step) % len;
            if self.entries[i].name.to_lowercase().starts_with(&lower) {
                return Some(i as u32);
            }
        }
        None
    }

    // -- sorting -----------------------------------------------------------

    /// Sort in place. Directories always come first regardless of the key,
    /// which is what every file manager does and what the old pure-alphabetical
    /// sort got wrong.
    pub fn sort(&mut self) {
        let key = self.sort_key;
        let order = self.sort_order;
        self.entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }
            let cmp = match key {
                SortKey::Name => natural_cmp(&a.name, &b.name),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Date => a.modified.cmp(&b.modified),
            };
            // Ties fall back to name so the order is stable and predictable.
            let cmp = cmp.then_with(|| natural_cmp(&a.name, &b.name));
            match order {
                SortOrder::Asc => cmp,
                SortOrder::Desc => cmp.reverse(),
            }
        });
    }

    /// Click on a column header: same key flips direction, new key starts ascending.
    pub fn apply_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_order = self.sort_order.flipped();
        } else {
            self.sort_key = key;
            self.sort_order = SortOrder::Asc;
        }
        self.resort_preserving_selection();
    }

    fn resort_preserving_selection(&mut self) {
        let sel: HashSet<String> = self
            .selected_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        let cursor_name = self.cursor_entry().map(|e| e.name.clone());
        self.sort();
        self.restore_selection(&sel, cursor_name.as_deref());
    }

    fn restore_selection(&mut self, names: &HashSet<String>, cursor_name: Option<&str>) {
        self.selected.clear();
        self.cursor = None;
        for (i, e) in self.entries.iter().enumerate() {
            if names.contains(&e.name) {
                self.selected.insert(i as u32);
            }
            if Some(e.name.as_str()) == cursor_name {
                self.cursor = Some(i as u32);
            }
        }
        self.anchor = self.cursor;
    }

    // -- loading -----------------------------------------------------------

    /// Replace the contents for a *new* directory: selection and scroll reset.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.apply_hidden_filter();
        self.sort();
        self.scroll_offset = 0;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
    }

    /// Replace the contents for the *same* directory (a refresh), keeping the
    /// selection, cursor, and scroll position on the entries that still exist.
    pub fn refresh_entries(&mut self, entries: Vec<FileEntry>) {
        let sel: HashSet<String> = self
            .selected_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        let cursor_name = self.cursor_entry().map(|e| e.name.clone());
        let scroll = self.scroll_offset;

        self.entries = entries;
        self.apply_hidden_filter();
        self.sort();
        self.restore_selection(&sel, cursor_name.as_deref());
        self.scroll_offset = scroll.min(self.entries.len() as u32);
    }

    /// Select a single entry by name, if present. Used when navigating up so
    /// the folder just left is highlighted.
    pub fn select_by_name(&mut self, name: &str, client_height: u32) {
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.select(i as u32, SelectMode::Replace);
            self.ensure_visible(i as u32, client_height);
        }
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
    }

    fn apply_hidden_filter(&mut self) {
        if !self.show_hidden {
            self.entries.retain(|e| !e.is_hidden);
        }
    }
}

/// Compare names the way a person reads them, so `file2` sorts before `file10`.
/// Falls back to case-insensitive lexicographic for everything else.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let na = take_number(&mut ai);
                    let nb = take_number(&mut bi);
                    match na.cmp(&nb) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    match la.cmp(&lb) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> u128 {
    let mut n: u128 = 0;
    while let Some(c) = it.peek() {
        if let Some(d) = c.to_digit(10) {
            // Saturate rather than wrap on absurdly long digit runs.
            n = n.saturating_mul(10).saturating_add(d as u128);
            it.next();
        } else {
            break;
        }
    }
    n
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
            extension: None,
        }
    }

    fn sized(name: &str, size: u64) -> FileEntry {
        FileEntry {
            size,
            ..entry(name, false)
        }
    }

    fn list_of(names: &[(&str, bool)]) -> FileList {
        let mut l = FileList::default();
        l.row_height = 24;
        l.set_entries(names.iter().map(|(n, d)| entry(n, *d)).collect());
        l
    }

    #[test]
    fn test_visible_range() {
        let list = list_of(&[("a", true), ("b", true), ("c", false), ("d", false), ("e", false)]);
        // 5 entries, 24px rows, 72px tall = 3 whole rows plus one partial.
        let (s, e) = list.visible_range(72);
        assert_eq!(s, 0);
        assert!(e >= 3);
    }

    #[test]
    fn test_scroll_by_clamps() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false), ("d", false), ("e", false)]);
        let h = 72u32; // 3 visible
        list.scroll_by(1, h);
        assert_eq!(list.scroll_offset, 1);
        list.scroll_by(10, h);
        assert_eq!(list.scroll_offset, 2); // max is 5 - 3
        list.scroll_by(-10, h);
        assert_eq!(list.scroll_offset, 0);
    }

    #[test]
    fn test_directories_sort_first() {
        let list = list_of(&[
            ("zeta.txt", false),
            ("alpha.txt", false),
            ("src", true),
            ("docs", true),
        ]);
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["docs", "src", "alpha.txt", "zeta.txt"]);
    }

    #[test]
    fn test_directories_stay_first_when_sorting_by_size() {
        let mut list = FileList::default();
        list.set_entries(vec![
            sized("big.bin", 9_000),
            entry("folder", true),
            sized("small.bin", 10),
        ]);
        list.apply_sort(SortKey::Size);
        assert!(list.entries[0].is_dir, "folders must lead every sort");
    }

    #[test]
    fn test_natural_number_ordering() {
        let list = list_of(&[("file10.txt", false), ("file2.txt", false), ("file1.txt", false)]);
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["file1.txt", "file2.txt", "file10.txt"]);
    }

    #[test]
    fn test_first_down_selects_row_zero() {
        // Regression: the old handler did unwrap_or(0) + 1 and skipped entry 0.
        let mut list = list_of(&[("a", false), ("b", false), ("c", false)]);
        assert_eq!(list.cursor(), None);
        assert_eq!(list.move_cursor(1, SelectMode::Replace), Some(0));
        assert!(list.is_selected(0));
    }

    #[test]
    fn test_first_up_selects_last_row() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false)]);
        assert_eq!(list.move_cursor(-1, SelectMode::Replace), Some(2));
    }

    #[test]
    fn test_ensure_visible_scrolls_minimally() {
        // Regression: the old Up handler re-centred the viewport on every press.
        let mut list = FileList::default();
        list.row_height = 10;
        list.set_entries((0..100).map(|i| entry(&format!("f{:03}", i), false)).collect());
        let h = 100; // 10 visible

        list.scroll_to(50, h);
        list.ensure_visible(49, h);
        assert_eq!(list.scroll_offset, 49, "should step up by exactly one row");

        list.scroll_to(50, h);
        list.ensure_visible(59, h);
        assert_eq!(list.scroll_offset, 50, "already visible: must not move");

        list.scroll_to(50, h);
        list.ensure_visible(60, h);
        assert_eq!(list.scroll_offset, 51, "should step down by exactly one row");
    }

    #[test]
    fn test_range_selection() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false), ("d", false)]);
        list.select(1, SelectMode::Replace);
        list.select(3, SelectMode::Range);
        assert_eq!(list.selection_count(), 3);
        assert!(list.is_selected(1) && list.is_selected(2) && list.is_selected(3));
        assert!(!list.is_selected(0));
    }

    #[test]
    fn test_range_selection_shrinks_from_same_anchor() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false), ("d", false)]);
        list.select(0, SelectMode::Replace);
        list.select(3, SelectMode::Range);
        assert_eq!(list.selection_count(), 4);
        list.select(1, SelectMode::Range);
        assert_eq!(list.selection_count(), 2, "anchor stays put so the range shrinks");
    }

    #[test]
    fn test_toggle_selection() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false)]);
        list.select(0, SelectMode::Replace);
        list.select(2, SelectMode::Toggle);
        assert_eq!(list.selection_count(), 2);
        list.select(2, SelectMode::Toggle);
        assert_eq!(list.selection_count(), 1);
    }

    #[test]
    fn test_refresh_preserves_selection_and_cursor() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false)]);
        list.select(1, SelectMode::Replace);
        // "b" survives, a new file appears, "c" is gone.
        list.refresh_entries(vec![entry("a", false), entry("b", false), entry("d", false)]);
        assert_eq!(list.selection_count(), 1);
        assert_eq!(list.selected_entries()[0].name, "b");
        assert_eq!(list.cursor_entry().map(|e| e.name.as_str()), Some("b"));
    }

    #[test]
    fn test_set_entries_resets_selection() {
        let mut list = list_of(&[("a", false), ("b", false)]);
        list.select(1, SelectMode::Replace);
        list.set_entries(vec![entry("x", false)]);
        assert_eq!(list.selection_count(), 0);
        assert_eq!(list.cursor(), None);
    }

    #[test]
    fn test_hidden_entries_filtered_unless_shown() {
        let mut list = FileList::default();
        let hidden = FileEntry {
            is_hidden: true,
            ..entry("secret.sys", false)
        };
        list.set_entries(vec![entry("visible.txt", false), hidden.clone()]);
        assert_eq!(list.entries.len(), 1);

        list.set_show_hidden(true);
        list.set_entries(vec![entry("visible.txt", false), hidden]);
        assert_eq!(list.entries.len(), 2);
    }

    #[test]
    fn test_apply_sort_toggles_direction() {
        let mut list = list_of(&[("a", false), ("b", false)]);
        list.apply_sort(SortKey::Name);
        assert_eq!(list.sort_order, SortOrder::Desc);
        assert_eq!(list.entries[0].name, "b");
        list.apply_sort(SortKey::Name);
        assert_eq!(list.sort_order, SortOrder::Asc);
        assert_eq!(list.entries[0].name, "a");
    }

    #[test]
    fn test_find_prefix_wraps() {
        let mut list = list_of(&[("apple", false), ("banana", false), ("avocado", false)]);
        // sorted: apple, avocado, banana
        list.select(1, SelectMode::Replace); // on "avocado"
        assert_eq!(list.find_prefix("b"), Some(2));
        list.select(2, SelectMode::Replace); // on "banana"
        assert_eq!(list.find_prefix("a"), Some(0), "search wraps to the top");
    }

    #[test]
    fn test_select_all() {
        let mut list = list_of(&[("a", false), ("b", false), ("c", false)]);
        list.select_all();
        assert_eq!(list.selection_count(), 3);
    }
}
