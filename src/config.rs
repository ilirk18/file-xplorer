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

#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    /// Panes on screen, 1..=MAX_PANES.
    pub pane_count: usize,
    pub theme_dark: bool,
    pub sidebar_visible: bool,
    pub show_hidden: bool,
    /// One fraction of the body width per divider. Fractions rather than
    /// pixels so a window resize keeps the panes in proportion; an empty or
    /// wrong-length list means "share the width evenly".
    pub splits: Vec<f32>,

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
}

/// Sentinel for "no saved position". A real window can legitimately sit at a
/// negative coordinate on a left-hand monitor, so 0 would not do.
pub const UNSET: i32 = i32::MIN;

impl Default for Config {
    fn default() -> Self {
        Self {
            pane_count: 1,
            theme_dark: true,
            sidebar_visible: true,
            show_hidden: false,
            splits: Vec::new(),
            win_x: UNSET,
            win_y: UNSET,
            win_w: 1200,
            win_h: 760,
            maximized: false,
            sync_scroll: false,
            compare: false,
            tabs: vec![Vec::new(); MAX_PANES],
            active_tab: vec![0; MAX_PANES],
        }
    }
}

/// Fractions as plain text: "0.33,0.66". Anything unparseable yields an empty
/// list, which the layout reads as "share the width evenly".
fn join_splits(splits: &[f32]) -> String {
    splits
        .iter()
        .map(|f| format!("{:.4}", f))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_splits(value: &str) -> Vec<f32> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    let parsed: Option<Vec<f32>> = value.split(',').map(|p| p.trim().parse().ok()).collect();
    // A fraction outside 0..1 would put a divider off screen; drop the lot and
    // let the layout even things out rather than restoring something unusable.
    match parsed {
        Some(v) if v.iter().all(|f| (0.0..=1.0).contains(f)) => v,
        _ => Vec::new(),
    }
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
             pane_count={}\n\
             theme_dark={}\n\
             sidebar_visible={}\n\
             show_hidden={}\n\
             splits={}\n\
             win_x={}\n\
             win_y={}\n\
             win_w={}\n\
             win_h={}\n\
             maximized={}\n\
             sync_scroll={}\n\
             compare={}\n{}",
            self.pane_count,
            self.theme_dark,
            self.sidebar_visible,
            self.show_hidden,
            join_splits(&self.splits),
            self.win_x,
            self.win_y,
            self.win_w,
            self.win_h,
            self.maximized,
            self.sync_scroll,
            self.compare,
            self.session_text()
        )
    }

    /// One `tab<n>=` line per open tab, in order, plus the active index.
    /// Repeated keys rather than a delimiter, because a path can contain very
    /// nearly any character and inventing an escape would be the third format
    /// in this file.
    fn session_text(&self) -> String {
        let mut out = String::new();
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
                "pane_count" => c.pane_count = value.parse().unwrap_or(c.pane_count),
                // Files written before multi-pane only knew one flag.
                "dual" => {
                    if value.parse().unwrap_or(false) {
                        c.pane_count = 2;
                    }
                }
                "theme_dark" => c.theme_dark = value.parse().unwrap_or(c.theme_dark),
                "sidebar_visible" => {
                    c.sidebar_visible = value.parse().unwrap_or(c.sidebar_visible)
                }
                "show_hidden" => c.show_hidden = value.parse().unwrap_or(c.show_hidden),
                "splits" => c.splits = parse_splits(value),
                "win_x" => c.win_x = value.parse().unwrap_or(c.win_x),
                "win_y" => c.win_y = value.parse().unwrap_or(c.win_y),
                "win_w" => c.win_w = value.parse().unwrap_or(c.win_w),
                "win_h" => c.win_h = value.parse().unwrap_or(c.win_h),
                "maximized" => c.maximized = value.parse().unwrap_or(c.maximized),
                "sync_scroll" => c.sync_scroll = value.parse().unwrap_or(c.sync_scroll),
                "compare" => c.compare = value.parse().unwrap_or(c.compare),
                k => {
                    if let Some(i) = pane_index(k, "tab") {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c = Config {
            pane_count: 3,
            theme_dark: false,
            sidebar_visible: false,
            show_hidden: true,
            splits: vec![0.25, 0.75],
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
        assert_eq!(c.pane_count, 2);
    }

    #[test]
    fn a_split_outside_the_window_is_rejected_whole() {
        // 1.4 would put a divider past the right edge; the pair goes with it.
        assert!(Config::from_text("splits=0.5,1.4\n").splits.is_empty());
        assert_eq!(Config::from_text("splits=0.5\n").splits, vec![0.5]);
    }

    #[test]
    fn the_old_dual_flag_still_opens_two_panes() {
        assert_eq!(Config::from_text("dual=true\n").pane_count, 2);
        assert_eq!(Config::from_text("dual=false\n").pane_count, 1);
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
    fn comments_and_whitespace_are_tolerated() {
        let c = Config::from_text("# hi\n\n  theme_dark = false  \n");
        assert!(!c.theme_dark);
    }
}
