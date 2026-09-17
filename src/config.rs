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

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    pub dual: bool,
    pub theme_dark: bool,
    pub sidebar_visible: bool,
    pub show_hidden: bool,
    /// Divider position in physical pixels. 0 means "never set, centre it".
    /// Clamped on load anyway, so a value from a different monitor is harmless.
    pub split_x: i32,

    /// Restored window box. `win_x == UNSET` means "let Windows place it".
    pub win_x: i32,
    pub win_y: i32,
    pub win_w: i32,
    pub win_h: i32,
    pub maximized: bool,

    pub sync_scroll: bool,
    pub compare: bool,
}

/// Sentinel for "no saved position". A real window can legitimately sit at a
/// negative coordinate on a left-hand monitor, so 0 would not do.
pub const UNSET: i32 = i32::MIN;

impl Default for Config {
    fn default() -> Self {
        Self {
            dual: false,
            theme_dark: true,
            sidebar_visible: true,
            show_hidden: false,
            split_x: 0,
            win_x: UNSET,
            win_y: UNSET,
            win_w: 1200,
            win_h: 760,
            maximized: false,
            sync_scroll: false,
            compare: false,
        }
    }
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
             dual={}\n\
             theme_dark={}\n\
             sidebar_visible={}\n\
             show_hidden={}\n\
             split_x={}\n\
             win_x={}\n\
             win_y={}\n\
             win_w={}\n\
             win_h={}\n\
             maximized={}\n\
             sync_scroll={}\n\
             compare={}\n",
            self.dual,
            self.theme_dark,
            self.sidebar_visible,
            self.show_hidden,
            self.split_x,
            self.win_x,
            self.win_y,
            self.win_w,
            self.win_h,
            self.maximized,
            self.sync_scroll,
            self.compare
        )
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
                "dual" => c.dual = value.parse().unwrap_or(c.dual),
                "theme_dark" => c.theme_dark = value.parse().unwrap_or(c.theme_dark),
                "sidebar_visible" => {
                    c.sidebar_visible = value.parse().unwrap_or(c.sidebar_visible)
                }
                "show_hidden" => c.show_hidden = value.parse().unwrap_or(c.show_hidden),
                "split_x" => c.split_x = value.parse().unwrap_or(c.split_x),
                "win_x" => c.win_x = value.parse().unwrap_or(c.win_x),
                "win_y" => c.win_y = value.parse().unwrap_or(c.win_y),
                "win_w" => c.win_w = value.parse().unwrap_or(c.win_w),
                "win_h" => c.win_h = value.parse().unwrap_or(c.win_h),
                "maximized" => c.maximized = value.parse().unwrap_or(c.maximized),
                "sync_scroll" => c.sync_scroll = value.parse().unwrap_or(c.sync_scroll),
                "compare" => c.compare = value.parse().unwrap_or(c.compare),
                _ => {}
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
            dual: true,
            theme_dark: false,
            sidebar_visible: false,
            show_hidden: true,
            split_x: 812,
            win_x: -1400,
            win_y: 20,
            win_w: 900,
            win_h: 700,
            maximized: true,
            sync_scroll: true,
            compare: true,
        };
        assert_eq!(Config::from_text(&c.to_text()), c);
    }

    #[test]
    fn empty_or_missing_gives_defaults() {
        assert_eq!(Config::from_text(""), Config::default());
    }

    #[test]
    fn garbage_keeps_defaults_rather_than_failing() {
        let c = Config::from_text("dual=yes please\nsplit_x=abc\nnonsense\n=\n");
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
        let c = Config::from_text("dual=true\nfuture_option=42\n");
        assert!(c.dual);
    }

    #[test]
    fn comments_and_whitespace_are_tolerated() {
        let c = Config::from_text("# hi\n\n  theme_dark = false  \n");
        assert!(!c.theme_dark);
    }
}
