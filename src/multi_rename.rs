// Renaming several files at once, in place, with one caret in all of them.
//
// Every selected row becomes an editable name and every keystroke lands in all
// of them at the same offset. Delete five characters and five characters leave
// each name; type a date and every name gains it in the same place. The list
// *is* the preview, which is the whole point: a pattern dialog can only show
// you what it thinks will happen.
//
// Hand-drawn rather than a Win32 EDIT per row, because only one control can
// have focus and the keystrokes have to reach all of them. That trade costs
// IME and the control's own undo; `batch_rename`'s dialog is still there for
// anything this cannot do, and one row still gets the real EDIT.
//
// Offsets are shared and unclamped: each row clamps for itself, so a short name
// is not dragged along by a long one and the caret comes back to where it was
// when the text grows again.

use crate::batch_rename::{self, split_name};
use crate::layout::PaneId;

/// Where a caret movement lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Left,
    Right,
    Home,
    End,
}

pub struct MultiRename {
    pub pid: PaneId,
    /// Absolute row indices in the pane's listing, ascending.
    pub rows: Vec<u32>,
    /// Live text, one per row, as characters because every offset here is one.
    names: Vec<Vec<char>>,
    /// What each row was called, and what its tokens expand to.
    items: Vec<batch_rename::Item>,
    caret: usize,
    anchor: usize,
}

impl MultiRename {
    /// Start editing `rows` of `pid`, with the stem selected.
    ///
    /// The stem of the *first* row, because the offsets are shared: these are
    /// one selection shown in several places, not several selections.
    pub fn begin(rows: Vec<u32>, pid: PaneId, entries: &[crate::fs::FileEntry]) -> Option<Self> {
        let mut names = Vec::new();
        let mut items = Vec::new();
        let mut kept = Vec::new();
        for r in rows {
            let Some(e) = entries.get(r as usize) else {
                continue;
            };
            names.push(e.name.chars().collect());
            items.push(batch_rename::Item {
                name: e.name.clone(),
                date: crate::fs::format_filetime(e.modified)
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            });
            kept.push(r);
        }
        if kept.is_empty() {
            return None;
        }
        let stem = split_name(&items[0].name).0.chars().count();
        Some(MultiRename {
            pid,
            rows: kept,
            names,
            items,
            caret: stem,
            anchor: 0,
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.rows.len()
    }

    /// The text of the row at `index` within this edit, for drawing.
    pub fn text(&self, index: usize) -> String {
        self.names.get(index).map(|n| n.iter().collect()).unwrap_or_default()
    }

    /// Which of the edited rows a listing row is, if any.
    pub fn index_of(&self, row: u32) -> Option<usize> {
        self.rows.iter().position(|r| *r == row)
    }

    /// The selection as (start, end), in characters, unclamped.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// The selection clamped to one row's length: what to draw for it.
    pub fn selection_in(&self, index: usize) -> (usize, usize) {
        let len = self.names.get(index).map(|n| n.len()).unwrap_or(0);
        let (a, b) = self.selection();
        (a.min(len), b.min(len))
    }

    pub fn caret_in(&self, index: usize) -> usize {
        let len = self.names.get(index).map(|n| n.len()).unwrap_or(0);
        self.caret.min(len)
    }

    fn longest(&self) -> usize {
        self.names.iter().map(|n| n.len()).max().unwrap_or(0)
    }

    /// Replace the selection in every row with `text`.
    pub fn insert(&mut self, text: &str) {
        let (a, b) = self.selection();
        let insert: Vec<char> = text.chars().collect();
        for n in &mut self.names {
            let from = a.min(n.len());
            let to = b.min(n.len());
            n.splice(from..to, insert.iter().copied());
        }
        self.caret = a + insert.len();
        self.anchor = self.caret;
    }

    /// Insert something different into each row — a date, a number, an id.
    ///
    /// The value goes in expanded rather than as a token, so the row shows what
    /// it will be called rather than what it was asked for.
    pub fn insert_token(&mut self, token: &str) {
        let (a, b) = self.selection();
        let values: Vec<Vec<char>> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| batch_rename::expand(token, item, i + 1).chars().collect())
            .collect();
        let mut end = a;
        for (n, v) in self.names.iter_mut().zip(&values) {
            let from = a.min(n.len());
            let to = b.min(n.len());
            n.splice(from..to, v.iter().copied());
            end = end.max(a + v.len());
        }
        // Rows can gain different amounts; the caret lands after the longest,
        // which is the only position that is past every insertion.
        self.caret = end;
        self.anchor = end;
    }

    pub fn backspace(&mut self) {
        let (a, b) = self.selection();
        if a != b {
            self.insert("");
            return;
        }
        if a == 0 {
            return;
        }
        for n in &mut self.names {
            if a <= n.len() {
                n.remove(a - 1);
            }
        }
        self.caret = a - 1;
        self.anchor = self.caret;
    }

    pub fn delete(&mut self) {
        let (a, b) = self.selection();
        if a != b {
            self.insert("");
            return;
        }
        for n in &mut self.names {
            if a < n.len() {
                n.remove(a);
            }
        }
        self.caret = a;
        self.anchor = a;
    }

    pub fn move_caret(&mut self, to: Move, extend: bool) {
        let (a, b) = self.selection();
        let target = match to {
            // Collapsing a selection lands on its edge rather than moving past
            // it, which is what every text field does.
            Move::Left if !extend && a != b => a,
            Move::Right if !extend && a != b => b,
            Move::Left => self.caret.saturating_sub(1),
            Move::Right => (self.caret + 1).min(self.longest()),
            Move::Home => 0,
            Move::End => self.longest(),
        };
        self.caret = target;
        if !extend {
            self.anchor = target;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.longest();
    }

    /// Select the stem in every row, which is where a rename usually starts.
    pub fn select_stem(&mut self) {
        self.anchor = 0;
        self.caret = split_name(&self.items[0].name).0.chars().count();
    }

    /// The selected text of the first row: what Ctrl+C would copy.
    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection_in(0);
        self.names[0][a..b].iter().collect()
    }

    /// (old name, new name) for every row that would actually change.
    ///
    /// An empty or illegal name is an error rather than a rename, so it comes
    /// back as `Err` with the first one that is wrong: half a batch applied is
    /// worse than none of it.
    pub fn plan(&self) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for (item, chars) in self.items.iter().zip(&self.names) {
            let new: String = chars.iter().collect();
            if new == item.name {
                continue;
            }
            if let Err(e) = crate::fs::validate_file_name(&new) {
                return Err(format!("\u{201c}{}\u{201d}: {}", new, e.message()));
            }
            out.push((item.name.clone(), new));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(names: &[&str]) -> MultiRename {
        MultiRename {
            pid: PaneId(0),
            rows: (0..names.len() as u32).collect(),
            names: names.iter().map(|n| n.chars().collect()).collect(),
            items: names
                .iter()
                .map(|n| batch_rename::Item {
                    name: (*n).to_string(),
                    date: "2026-09-15".into(),
                })
                .collect(),
            caret: 0,
            anchor: 0,
        }
    }

    fn texts(m: &MultiRename) -> Vec<String> {
        (0..m.len()).map(|i| m.text(i)).collect()
    }

    #[test]
    fn typing_lands_in_every_name_at_the_same_offset() {
        let mut m = editor(&["01 - Wallpaper.jpg", "02 - Wallpaper.jpg"]);
        m.anchor = 0;
        m.caret = 5; // "01 - "
        m.insert("2024-11-25 ");
        assert_eq!(
            texts(&m),
            ["2024-11-25 Wallpaper.jpg", "2024-11-25 Wallpaper.jpg"]
        );
        // And the caret is past what was typed, in every row at once.
        assert_eq!(m.selection(), (11, 11));
    }

    #[test]
    fn a_short_name_clamps_instead_of_being_dragged_along() {
        // The offsets are shared, so a row that is too short to reach them
        // takes what it can and the rest is left alone.
        let mut m = editor(&["a.txt", "a-very-long-name.txt"]);
        m.anchor = 6;
        m.caret = 10;
        m.insert("X");
        assert_eq!(texts(&m), ["a.txtX", "a-veryXg-name.txt"]);
    }

    #[test]
    fn backspace_takes_one_character_from_each() {
        let mut m = editor(&["ab.txt", "cd.txt"]);
        m.caret = 2;
        m.anchor = 2;
        m.backspace();
        assert_eq!(texts(&m), ["a.txt", "c.txt"]);
        assert_eq!(m.selection(), (1, 1));

        // At the start there is nothing to take, and nothing moves.
        m.caret = 0;
        m.anchor = 0;
        m.backspace();
        assert_eq!(texts(&m), ["a.txt", "c.txt"]);
    }

    #[test]
    fn a_token_gives_each_row_its_own_value() {
        let mut m = editor(&["one.txt", "two.txt", "three.txt"]);
        m.anchor = 0;
        m.caret = 0;
        m.insert_token("{##} ");
        assert_eq!(
            texts(&m),
            ["01 one.txt", "02 two.txt", "03 three.txt"]
        );
        // Past the longest insertion, so it is past every one of them.
        assert_eq!(m.selection(), (3, 3));
    }

    #[test]
    fn a_plan_leaves_out_what_did_not_change_and_refuses_what_cannot_be_a_name() {
        let mut m = editor(&["keep.txt", "change.txt"]);
        m.anchor = 0;
        m.caret = 6;
        // Only the second row's stem is six characters, so only it changes.
        m.insert("change");
        let plan = m.plan().expect("legal");
        assert_eq!(plan.len(), 1, "{:?}", plan);
        assert_eq!(plan[0].0, "keep.txt");

        let mut bad = editor(&["a.txt"]);
        bad.select_all();
        bad.insert("no/slashes");
        assert!(bad.plan().is_err(), "a slash is not a name");
    }

    #[test]
    fn moving_left_off_a_selection_lands_on_its_edge() {
        let mut m = editor(&["abcd.txt"]);
        m.anchor = 2;
        m.caret = 5;
        m.move_caret(Move::Left, false);
        assert_eq!(m.selection(), (2, 2));

        m.anchor = 2;
        m.caret = 5;
        m.move_caret(Move::Right, false);
        assert_eq!(m.selection(), (5, 5));

        // Shift keeps the anchor where the last move left it, which is what
        // makes a selection grow from here rather than from where it was.
        m.move_caret(Move::Right, true);
        assert_eq!(m.selection(), (5, 6));
    }
}
