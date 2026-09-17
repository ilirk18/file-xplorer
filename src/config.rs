// Settings that survive a restart.
//
// A handful of scalars in a `key=value` text file. Deliberately not serde: that
// is a large build-time dependency for five booleans, and a plain text file is
// something you can open and fix by hand when it goes wrong.
//
// Parsing and formatting are separated from the file I/O so they can be tested
// without touching the disk. Anything unreadable falls back to defaults rather
// than failing: losing a preference is not worth refusing to start.

use std::path::PathBuf;

use crate::layout::MAX_PANES;

/// Push `path` onto a most-recent-first history: no duplicates, bounded.
/// Comparison is case-insensitive because Windows paths are.
pub fn push_recent(history: &mut Vec<String>, path: &str) {
    if path.is_empty() {
        return;
    }
    history.retain(|p| !p.eq_ignore_ascii_case(path));
    history.insert(0, path.to_string());
    history.truncate(MAX_RECENT);
}

/// How many folders keep their own sort and view. Only a deliberate change
/// writes one, so this is a few years of them.
pub const MAX_FOLDER_VIEWS: usize = 200;

/// How many visited folders are kept. Enough to cover a session's worth of
/// jumping about, short enough that the list stays pickable.
pub const MAX_RECENT: usize = 20;

#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    /// The pane tree, as text. Empty means "one pane", and the older
    /// `pane_count` key is still read so a settings file from before trees
    /// opens the way it was left.
    pub layout: String,
    /// Theme by name, as `theme.rs` spells it. A name rather than an index so
     /// the file still means the same thing when the table gains a row.
    pub theme: String,
    /// UI font family, and its size as a percentage. A family the system does
    /// not have falls back at draw time rather than failing to start.
    pub font: String,
    pub font_size: i32,
    /// Row height as a percentage. Text size already moves rows; this is the
    /// part on top of it that is taste.
    pub density: i32,
    /// Size folders automatically when a folder opens. Off by default: a
    /// recursive walk of a drive is how a file manager earns a reputation for
    /// hanging, and the on-request command is still there.
    pub folder_sizes: bool,
    pub sidebar_visible: bool,
    pub inspector: bool,
    pub command_bar: bool,
    /// Icon edge in DIPs, or 0 for the details list.
    pub icons: i32,
    pub show_hidden: bool,
    /// Restored window box. `win_x == UNSET` means "let Windows place it".
    pub win_x: i32,
    pub win_y: i32,
    pub win_w: i32,
    pub win_h: i32,
    pub maximized: bool,

    pub sync_scroll: bool,
    pub compare: bool,

    /// Open tabs per pane, and which of them was active. One entry per pane,
    /// always `MAX_PANES` long; an empty list means "start at the working
    /// directory".
    pub tabs: Vec<Vec<String>>,
    pub active_tab: Vec<usize>,

    /// Folders pinned into the sidebar's Places, in the order they were added.
    pub pins: Vec<String>,
    /// Shell context-menu verbs hoisted to the top of the menu, by the text
    /// the menu showed. Matching on text is approximate once the machine
    /// changes language, which is honest for a convenience feature.
    pub pin_actions: Vec<String>,
    /// How each folder was last looked at, most recently changed first, as
    /// (lowercased path, sort key, ascending, icon size).
    pub folder_views: Vec<(String, u8, bool, i32)>,
    /// Named pane trees, as (name, the same text `layout` uses). A saved
    /// layout is a layout, so it is stored the way one already is.
    pub saved_layouts: Vec<(String, String)>,
    /// Shortcuts that differ from the defaults, as (chord text, command
    /// label). An empty chord means the command has had its shortcut removed.
    pub binds: Vec<(String, String)>,
    /// Folders visited, most recent first. Bounded by `MAX_RECENT`: a history
    /// is a shortcut, not a log.
    pub recent: Vec<String>,
    /// Sort key and direction per pane, as `(key, ascending)`.
    pub sort: Vec<(u8, bool)>,
    /// Type, Size and Date column widths in DIPs — not pixels, so a settings
    /// file carried to a different monitor still means the same thing.
    pub col_widths: [i32; 3],
}

/// Sentinel for "no saved position". A real window can legitimately sit at a
/// negative coordinate on a left-hand monitor, so 0 would not do.
pub const UNSET: i32 = i32::MIN;

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: String::new(),
            // No saved preference means "whatever Windows is set to". Once the
            // user toggles the theme the key is written and wins from then on.
            // No saved preference means "whatever Windows is set to".
            theme: if crate::theme::system_dark() { "Dark" } else { "Light" }.to_string(),
            font: crate::renderer::DEFAULT_FONT.to_string(),
            font_size: 100,
            density: 100,
            folder_sizes: false,
            sidebar_visible: true,
            inspector: false,
            command_bar: true,
            icons: 0,
            show_hidden: false,
            win_x: UNSET,
            win_y: UNSET,
            win_w: 1200,
            win_h: 760,
            maximized: false,
            sync_scroll: false,
            compare: false,
            tabs: vec![Vec::new(); MAX_PANES],
            active_tab: vec![0; MAX_PANES],
            pins: Vec::new(),
            pin_actions: Vec::new(),
            saved_layouts: Vec::new(),
            folder_views: Vec::new(),
            recent: Vec::new(),
            binds: Vec::new(),
            sort: vec![(0, true); MAX_PANES],
            col_widths: [96, 84, 124],
        }
    }
}

/// Three widths, or the defaults. A column narrower than this is a sliver you
/// cannot grab again, and one wider than this hides the Name column.
fn parse_widths(value: &str, fallback: [i32; 3]) -> [i32; 3] {
    let parts: Vec<i32> = value.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.len() != 3 || parts.iter().any(|w| !(40..=400).contains(w)) {
        return fallback;
    }
    [parts[0], parts[1], parts[2]]
}

/// `tab2` -> Some(2), for a key in range. Anything else is an unknown key and
/// is ignored, which is what lets an older build read a newer file.
fn pane_index(key: &str, prefix: &str) -> Option<usize> {
    let i: usize = key.strip_prefix(prefix)?.parse().ok()?;
    (i < MAX_PANES).then_some(i)
}

fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("FileXplorer").join("settings.txt"))
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Self::from_text(&s))
            .unwrap_or_default()
    }

    /// Best effort: a settings file we cannot write is not worth a dialog.
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.to_text());
    }

    pub fn to_text(&self) -> String {
        format!(
            "# File Xplorer settings\n\
             layout={}\n\
             theme={}\n\
             font={}\n\
             font_size={}\n\
             density={}\n\
             folder_sizes={}\n\
             sidebar_visible={}\n\
             inspector={}\n\
             command_bar={}\n\
             icons={}\n\
             show_hidden={}\n\
             win_x={}\n\
             win_y={}\n\
             win_w={}\n\
             win_h={}\n\
             maximized={}\n\
             sync_scroll={}\n\
             compare={}\n\
             col_widths={},{},{}\n{}",
            self.layout,
            self.theme,
            self.font,
            self.font_size,
            self.density,
            self.folder_sizes,
            self.sidebar_visible,
            self.inspector,
            self.command_bar,
            self.icons,
            self.show_hidden,
            self.win_x,
            self.win_y,
            self.win_w,
            self.win_h,
            self.maximized,
            self.sync_scroll,
            self.compare,
            self.col_widths[0],
            self.col_widths[1],
            self.col_widths[2],
            self.session_text()
        )
    }

    /// One `tab<n>=` line per open tab, in order, plus the active index.
    /// Repeated keys rather than a delimiter, because a path can contain very
    /// nearly any character and inventing an escape would be the third format
    /// in this file.
    fn session_text(&self) -> String {
        let mut out = String::new();
        for p in &self.pins {
            out.push_str(&format!("pin={}\n", p));
        }
        for a in &self.pin_actions {
            out.push_str(&format!("pinaction={}\n", a));
        }
        for (path, key, asc, icons) in self.folder_views.iter().take(MAX_FOLDER_VIEWS) {
            out.push_str(&format!("folderview={}={},{},{}
", path, key, asc, icons));
        }
        for (name, tree) in &self.saved_layouts {
            out.push_str(&format!("savedlayout={}={}
", name, tree));
        }
        for (chord, label) in &self.binds {
            out.push_str(&format!("bind={}={}
", chord, label));
        }
        for p in self.recent.iter().take(MAX_RECENT) {
            out.push_str(&format!("recent={}\n", p));
        }
        for (i, (key, asc)) in self.sort.iter().enumerate() {
            if (*key, *asc) != (0, true) {
                out.push_str(&format!("sort{}={},{}\n", i, key, asc));
            }
        }
        for (i, paths) in self.tabs.iter().enumerate() {
            if paths.is_empty() {
                continue;
            }
            for p in paths {
                out.push_str(&format!("tab{}={}\n", i, p));
            }
            out.push_str(&format!("active{}={}\n", i, self.active_tab[i]));
        }
        out
    }

    pub fn from_text(text: &str) -> Self {
        let mut c = Self::default();
        // This file is meant to be editable by hand, and a Windows editor may
        // leave a byte-order mark on the front. Without this the first key
        // silently stops matching and that one setting appears not to stick.
        let text = text.trim_start_matches('\u{feff}');
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            // A bad value keeps the default; an unknown key is ignored, so an
            // older build can read a newer file.
            match key.trim() {
                "layout" => c.layout = value.to_string(),
                // Before there were trees. A count and a list of fractions
                // describe a row of columns, which is one shape of tree.
                "pane_count" => {
                    // Only a number this file actually contains becomes a
                    // layout: a garbled line leaves the default alone rather
                    // than quietly writing one pane over it.
                    if let (true, Ok(n)) = (c.layout.is_empty(), value.parse::<usize>()) {
                        c.layout = tree_to_text(&crate::layout::Node::columns(n));
                    }
                }
                // Files written before multi-pane only knew one flag.
                "dual" => {
                    if value.parse().unwrap_or(false) && c.layout.is_empty() {
                        c.layout = tree_to_text(&crate::layout::Node::columns(2));
                    }
                }
                "font" => {
                    if !value.is_empty() {
                        c.font = value.to_string();
                    }
                }
                "font_size" => c.font_size = value.parse().unwrap_or(c.font_size),
                "density" => c.density = value.parse().unwrap_or(c.density),
                "folder_sizes" => c.folder_sizes = value.parse().unwrap_or(c.folder_sizes),
                "theme" => {
                    if !value.is_empty() {
                        c.theme = value.to_string();
                    }
                }
                // Files written before themes had names.
                "theme_dark" => {
                    c.theme = match value.parse::<bool>() {
                        Ok(true) => "Dark".into(),
                        Ok(false) => "Light".into(),
                        Err(_) => c.theme,
                    }
                }
                "sidebar_visible" => {
                    c.sidebar_visible = value.parse().unwrap_or(c.sidebar_visible)
                }
                "inspector" => c.inspector = value.parse().unwrap_or(c.inspector),
                "command_bar" => c.command_bar = value.parse().unwrap_or(c.command_bar),
                "icons" => c.icons = value.parse().unwrap_or(c.icons),
                // What the icon view used to be called, when it was on or off
                // rather than a size. Read so an older settings file still
                // opens in the view it was left in.
                "grid" => {
                    if value.parse().unwrap_or(false) {
                        c.icons = crate::layout::DEFAULT_ICONS;
                    }
                }
                "show_hidden" => c.show_hidden = value.parse().unwrap_or(c.show_hidden),
                "win_x" => c.win_x = value.parse().unwrap_or(c.win_x),
                "win_y" => c.win_y = value.parse().unwrap_or(c.win_y),
                "win_w" => c.win_w = value.parse().unwrap_or(c.win_w),
                "win_h" => c.win_h = value.parse().unwrap_or(c.win_h),
                "maximized" => c.maximized = value.parse().unwrap_or(c.maximized),
                "sync_scroll" => c.sync_scroll = value.parse().unwrap_or(c.sync_scroll),
                "compare" => c.compare = value.parse().unwrap_or(c.compare),
                "pin" => {
                    if !value.is_empty() {
                        c.pins.push(value.to_string());
                    }
                }
                "pinaction" => {
                    if !value.is_empty() {
                        c.pin_actions.push(value.to_string());
                    }
                }
                // `folderview=<path>=<key>,<ascending>,<icons>`. The path
                // comes first and can hold anything, so as with `bind` the
                // split is at the first "=" and the fixed part is what follows.
                "folderview" => {
                    if c.folder_views.len() >= MAX_FOLDER_VIEWS {
                        continue;
                    }
                    if let Some((path, rest)) = value.split_once('=') {
                        let f: Vec<&str> = rest.split(',').collect();
                        if let (3, false) = (f.len(), path.is_empty()) {
                            if let (Ok(key), Ok(asc), Ok(icons)) =
                                (f[0].trim().parse(), f[1].trim().parse(), f[2].trim().parse())
                            {
                                c.folder_views.push((path.to_lowercase(), key, asc, icons));
                            }
                        }
                    }
                }
                // `savedlayout=<name>=<tree>`, split at the first "=" as
                // `bind` is: a name is free text and a tree never contains one.
                "savedlayout" => {
                    if let Some((name, tree)) = value.split_once('=') {
                        if !name.is_empty() && tree_checked(tree).is_some() {
                            c.saved_layouts.push((name.to_string(), tree.to_string()));
                        }
                    }
                }
                "bind" => {
                    // `bind=<chord>=<command label>`. The label can contain
                    // anything, so the split is at the *first* "=" only.
                    if let Some((chord, label)) = value.split_once('=') {
                        if !label.is_empty() {
                            c.binds.push((chord.to_string(), label.to_string()));
                        }
                    }
                }
                "recent" => {
                    if !value.is_empty() && c.recent.len() < MAX_RECENT {
                        c.recent.push(value.to_string());
                    }
                }
                "col_widths" => c.col_widths = parse_widths(value, c.col_widths),
                k => {
                    if let Some(i) = pane_index(k, "sort") {
                        if let Some((key, asc)) = value.split_once(',') {
                            c.sort[i] = (
                                key.trim().parse().unwrap_or(0),
                                asc.trim().parse().unwrap_or(true),
                            );
                        }
                    } else if let Some(i) = pane_index(k, "tab") {
                        if !value.is_empty() {
                            c.tabs[i].push(value.to_string());
                        }
                    } else if let Some(i) = pane_index(k, "active") {
                        c.active_tab[i] = value.parse().unwrap_or(0);
                    }
                }
            }
        }
        c
    }
}

/// Write a pane tree as text.
///
/// `V(0,0.5,1)` is two panes side by side; `H` stacks them. Readable and
/// editable, which is what the settings file has always promised, and small
/// enough that a saved layout is just another one of these.
pub fn tree_to_text(node: &crate::layout::Node) -> String {
    use crate::layout::Node;
    match node {
        Node::Leaf(id) => id.0.to_string(),
        Node::Split {
            vertical,
            ratio,
            a,
            b,
        } => format!(
            "{}({},{:.3},{})",
            if *vertical { "V" } else { "H" },
            tree_to_text(a),
            ratio,
            tree_to_text(b)
        ),
    }
}

/// Read one back. Anything malformed is one pane, because a settings file
/// nobody can parse should still start the app.
pub fn tree_from_text(text: &str) -> Option<crate::layout::Node> {
    let (node, rest) = parse_node(text.trim())?;
    rest.trim().is_empty().then_some(node)
}

fn parse_node(text: &str) -> Option<(crate::layout::Node, &str)> {
    use crate::layout::{Node, MAX_PANES};
    let text = text.trim_start();
    let vertical = match text.as_bytes().first()? {
        b'V' => true,
        b'H' => false,
        _ => {
            // A leaf: digits up to the next comma or bracket.
            let end = text
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(text.len());
            let id: usize = text[..end].parse().ok()?;
            if id >= MAX_PANES {
                return None;
            }
            return Some((Node::Leaf(crate::layout::PaneId(id)), &text[end..]));
        }
    };
    let rest = text.get(1..)?.strip_prefix('(')?;
    let (a, rest) = parse_node(rest)?;
    let rest = rest.trim_start().strip_prefix(',')?;
    let split = rest.find(',')?;
    let ratio: f32 = rest[..split].trim().parse().ok()?;
    let (b, rest) = parse_node(&rest[split + 1..])?;
    let rest = rest.trim_start().strip_prefix(')')?;
    Some((
        Node::Split {
            vertical,
            ratio: ratio.clamp(0.0, 1.0),
            a: Box::new(a),
            b: Box::new(b),
        },
        rest,
    ))
}

/// A tree that is safe to show. A tree naming the same pane twice is refused:
/// two leaves sharing an id would be two views of one pane's tabs, and every
/// operation on it would fight itself.
pub fn tree_checked(text: &str) -> Option<crate::layout::Node> {
    let node = tree_from_text(text)?;
    let mut seen: Vec<crate::layout::PaneId> = Vec::new();
    for id in node.leaves() {
        if seen.contains(&id) {
            return None;
        }
        seen.push(id);
    }
    Some(node)
}

impl Config {
    /// The saved tree, or one pane, because a settings file nobody can parse
    /// should still start the app.
    pub fn tree(&self) -> crate::layout::Node {
        tree_checked(&self.layout).unwrap_or_else(|| crate::layout::Node::columns(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_view_is_lowercased_and_a_malformed_one_is_dropped() {
        let c = Config::from_text(
            "folderview=C:\\Users=2,false,-72\nfolderview=C:\\bad=2,false\nfolderview=C:\\x=zz,true,0\n",
        );
        assert_eq!(c.folder_views, vec![("c:\\users".to_string(), 2, false, -72)]);
    }

    #[test]
    fn a_saved_layout_that_will_not_parse_is_dropped_on_the_way_in() {
        // A tree that will not parse, and one naming a pane twice, are both
        // refused on the way in: a layout you cannot switch to is worse in
        // the list than missing from it.
        let c = Config::from_text(
            "savedlayout=Bad=V(0,0.5,\nsavedlayout=Twice=V(0,0.500,0)\nsavedlayout=Good=1\n",
        );
        assert_eq!(c.saved_layouts, vec![("Good".to_string(), "1".to_string())]);
    }

    #[test]
    fn recent_is_most_recent_first_deduplicated_and_bounded() {
        // Plain names: push_recent cares about order, case and count, not about
        // what a path looks like.
        let mut h = Vec::new();
        push_recent(&mut h, "alpha");
        push_recent(&mut h, "beta");
        push_recent(&mut h, "ALPHA");
        assert_eq!(h, vec!["ALPHA".to_string(), "beta".to_string()]);

        push_recent(&mut h, "");
        assert_eq!(h.len(), 2, "an empty path is not a visit");

        for i in 0..MAX_RECENT + 5 {
            push_recent(&mut h, &format!("dir{}", i));
        }
        assert_eq!(h.len(), MAX_RECENT);
        assert_eq!(h[0], format!("dir{}", MAX_RECENT + 4));
    }

    #[test]
    fn a_pane_tree_round_trips_through_text() {
        use crate::layout::Node;
        for n in 1..=crate::layout::MAX_PANES {
            let text = tree_to_text(&Node::columns(n));
            let back = tree_from_text(&text).expect("parses");
            // Through the text, not through the ratios: three decimals cannot
            // hold a third exactly, and the file is the thing that has to be
            // stable, not the float.
            assert_eq!(tree_to_text(&back), text);
            assert_eq!(back.count(), n);
            assert_eq!(back.leaves(), Node::columns(n).leaves());
        }
        // Nesting and orientation survive too.
        let nested = tree_from_text("V(0,0.5,H(1,0.25,2))").expect("parses");
        assert_eq!(tree_to_text(&nested), "V(0,0.500,H(1,0.250,2))");
        assert_eq!(nested.count(), 3);
    }

    #[test]
    fn a_layout_nobody_can_parse_is_one_pane() {
        for bad in ["", "V(", "V(0,0.5)", "X(0,1)", "V(0,0.5,9)", "0)", "V(0,x,1)"] {
            assert!(tree_from_text(bad).is_none(), "{:?} should not parse", bad);
        }
        let cfg = Config {
            layout: "nonsense".into(),
            ..Config::default()
        };
        assert_eq!(cfg.tree().count(), 1);
    }

    #[test]
    fn a_tree_naming_one_pane_twice_is_refused() {
        // Two leaves with the same id would be two views of one pane's tabs,
        // and every operation on it would fight itself.
        let cfg = Config {
            layout: "V(1,0.5,1)".into(),
            ..Config::default()
        };
        assert_eq!(cfg.tree().count(), 1);
    }

    #[test]
    fn an_old_settings_file_still_opens_the_way_it_was_left() {
        let c = Config::from_text("pane_count=3\n");
        assert_eq!(c.tree().count(), 3, "three columns, as it always was");
        // An explicit layout wins over the old key whichever order they appear.
        let c = Config::from_text("layout=V(0,0.5,1)\npane_count=4\n");
        assert_eq!(c.tree().count(), 2);
    }

    #[test]
    fn round_trips() {
        let c = Config {
            layout: "V(0,0.5,V(1,0.5,2))".to_string(),
            theme: "Sepia".into(),
            font: "Consolas".into(),
            font_size: 125,
            density: 85,
            folder_sizes: true,
            sidebar_visible: false,
            inspector: true,
            command_bar: false,
            icons: 96,
            show_hidden: true,
            win_x: -1400,
            win_y: 20,
            win_w: 900,
            win_h: 700,
            maximized: true,
            sync_scroll: true,
            compare: true,
            tabs: vec![
                vec!["C:\\a".into(), "C:\\b c\\d=e".into()],
                Vec::new(),
                vec!["D:\\x".into()],
                Vec::new(),
            ],
            active_tab: vec![1, 0, 0, 0],
            folder_views: vec![("c:\\users\\foo".into(), 2, false, -72)],
            saved_layouts: vec![("Two up".into(), "V(0,0.500,1)".into())],
            pins: vec!["C:\\pinned".into()],
            // A label with a space and an ampersand: menu text has both.
            pin_actions: vec!["Open with".into(), "Scan && clean".into()],
            recent: vec![r"C:\Users".into(), r"D:\work".into()],
            binds: vec![
                ("Ctrl+Q".into(), "Refresh".into()),
                // An empty chord is how a removed shortcut is recorded, and it
                // has to survive the round trip or unbinding would not stick.
                (String::new(), "Toggle sidebar".into()),
            ],
            sort: vec![(2, false), (0, true), (0, true), (1, true)],
            col_widths: [110, 90, 130],
        };
        assert_eq!(Config::from_text(&c.to_text()), c);
    }

    #[test]
    fn empty_or_missing_gives_defaults() {
        assert_eq!(Config::from_text(""), Config::default());
    }

    #[test]
    fn garbage_keeps_defaults_rather_than_failing() {
        let c = Config::from_text("pane_count=lots\nsplits=abc\nnonsense\n=\n");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn negative_window_positions_survive() {
        // A window on a left-hand monitor has a negative x; it must round-trip.
        let c = Config::from_text("win_x=-1920\nwin_y=-40\n");
        assert_eq!((c.win_x, c.win_y), (-1920, -40));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // An older build must survive a file written by a newer one.
        let c = Config::from_text("pane_count=2\nfuture_option=42\n");
        assert_eq!(c.tree().count(), 2);
    }

    #[test]
    fn the_old_dual_flag_still_opens_two_panes() {
        assert_eq!(Config::from_text("dual=true\n").tree().count(), 2);
        assert_eq!(Config::from_text("dual=false\n").tree().count(), 1);
    }

    #[test]
    fn a_session_with_no_tabs_writes_nothing_for_it() {
        let c = Config::default();
        assert!(!c.to_text().contains("tab0="));
        assert_eq!(Config::from_text(&c.to_text()).tabs, c.tabs);
    }

    #[test]
    fn a_tab_line_for_a_pane_that_does_not_exist_is_ignored() {
        // MAX_PANES could shrink; a stale file must not index out of range.
        let c = Config::from_text("tab9=C:\\nope\ntab0=C:\\yes\n");
        assert_eq!(c.tabs[0], vec!["C:\\yes".to_string()]);
    }

    #[test]
    fn absurd_column_widths_are_rejected_as_a_set() {
        // A 5px column cannot be grabbed to widen it again, so the whole line
        // is thrown away rather than half-applied.
        assert_eq!(Config::from_text("col_widths=5,84,124\n").col_widths, [96, 84, 124]);
        assert_eq!(Config::from_text("col_widths=110,90,130\n").col_widths, [110, 90, 130]);
    }

    #[test]
    fn a_default_sort_is_not_written_out() {
        // Four "sort0=0,true" lines in every settings file would be noise.
        assert!(!Config::default().to_text().contains("sort"));
    }

    #[test]
    fn a_byte_order_mark_does_not_eat_the_first_setting() {
        let c = Config::from_text("\u{feff}pane_count=3\n");
        assert_eq!(c.tree().count(), 3);
    }

    #[test]
    fn comments_and_whitespace_are_tolerated() {
        let c = Config::from_text("# hi\n\n  theme_dark = false  \n");
        assert_eq!(c.theme, "Light", "a file from before themes had names");
    }
}
