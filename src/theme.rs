// Colour tokens.
//
// The dark palette targets the near-black, blue-accented look of modern
// file managers, with two deliberate improvements:
//
//  * Muted text sits at >= 4.5:1 against its background rather than the ~4:1
//    that is common in this style, so secondary columns stay readable.
//  * Surfaces step in even, perceptible increments (window < sidebar < pane <
//    hover < active) instead of collapsing into one flat black, which is what
//    makes panels legible without needing borders everywhere.

/// What Windows itself is set to, from the same registry value the shell reads.
///
/// Absent or unreadable means dark, which is this app's own default: a machine
/// that has never been themed should not get a surprise white window.
///
/// ponytail: read once at startup. Following a live theme change would mean
/// handling WM_SETTINGCHANGE / "ImmersiveColorSet"; add it if anyone notices.
pub fn system_dark() -> bool {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            PCWSTR(w!("AppsUseLightTheme").as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_err() || value == 0
}

/// One theme: a palette with a name and a side.
///
/// `dark` is not decoration — the window's caption is painted by Windows from
/// it, and a light palette under a dark caption reads as two title bars.
pub struct ThemeDef {
    pub name: &'static str,
    pub dark: bool,
    pub palette: Palette,
}

/// Every theme this app offers.
///
/// One table rather than a constant each, because the contrast tests at the
/// bottom of this file run over all of it: a new theme that fails AA cannot be
/// added without the suite saying so. That is the only reason this is a table.
///
/// Dark and Light come first and stay first — `toggled` is defined in terms
/// of the first theme of the opposite side.
pub const THEMES: &[ThemeDef] = &[
    ThemeDef { name: "Dark", dark: true, palette: DARK },
    ThemeDef { name: "Light", dark: false, palette: LIGHT },
    ThemeDef { name: "Midnight", dark: true, palette: MIDNIGHT },
    ThemeDef { name: "Sepia", dark: false, palette: SEPIA },
];

/// Which theme, by position in `THEMES`. Constructed only through `at` and
/// `by_name`, both of which refuse an index that is not in the table, so
/// `palette()` never has to wonder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Theme(usize);

impl Default for Theme {
    fn default() -> Self {
        Theme::DARK
    }
}

pub type Rgb = (f32, f32, f32);

/// An `Rgb` as GDI wants it: eight bits a channel, blue highest. Every module
/// that draws with GDI needs this, so it lives with the colours rather than
/// three times over.
pub fn colorref(c: Rgb) -> windows::Win32::Foundation::COLORREF {
    let q = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    windows::Win32::Foundation::COLORREF(q(c.0) | (q(c.1) << 8) | (q(c.2) << 16))
}

/// Convert an 0xRRGGBB literal to linear-ish sRGB floats.
/// `const` so the palettes below stay readable as hex.
const fn hex(v: u32) -> Rgb {
    (
        ((v >> 16) & 0xFF) as f32 / 255.0,
        ((v >> 8) & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
    )
}

#[derive(Clone, Copy)]
pub struct Palette {
    // Surfaces, darkest to lightest.
    pub window_bg: Rgb,
    pub sidebar_bg: Rgb,
    pub pane_bg: Rgb,
    pub tab_bar_bg: Rgb,
    pub toolbar_bg: Rgb,
    pub header_bg: Rgb,
    pub footer_bg: Rgb,
    pub field_bg: Rgb,

    // Rows.
    pub row_hover: Rgb,
    pub selection: Rgb,
    /// Selection in the pane that does not have focus.
    pub selection_inactive: Rgb,
    pub cursor_outline: Rgb,

    // Text.
    pub text: Rgb,
    pub text_muted: Rgb,
    pub text_faint: Rgb,
    pub text_on_selection: Rgb,

    // Chrome.
    pub tab_active: Rgb,
    pub tab_hover: Rgb,
    pub divider: Rgb,
    pub accent: Rgb,
    pub scrollbar_thumb: Rgb,
    pub scrollbar_thumb_hover: Rgb,

    // Drive capacity bars.
    pub capacity_track: Rgb,
    pub capacity_fill: Rgb,
    /// Used past ~90%: a full disk should read as a warning, not a decoration.
    pub capacity_full: Rgb,
}

const DARK: Palette = Palette {
    window_bg: hex(0x0F1013),
    sidebar_bg: hex(0x141518),
    pane_bg: hex(0x1A1B1F),
    tab_bar_bg: hex(0x141518),
    toolbar_bg: hex(0x17181C),
    header_bg: hex(0x17181C),
    footer_bg: hex(0x141518),
    field_bg: hex(0x0F1013),

    row_hover: hex(0x24262C),
    selection: hex(0x1580C4),
    selection_inactive: hex(0x2C3138),
    cursor_outline: hex(0x5AAEE8),

    text: hex(0xE8EAED),
    // 8.0:1 on pane_bg. The usual choice here is nearer #7A8088 (4.1:1).
    text_muted: hex(0xA3ABB5),
    text_faint: hex(0x6F767F),
    text_on_selection: hex(0xFFFFFF),

    tab_active: hex(0x1A1B1F),
    tab_hover: hex(0x1E2024),
    divider: hex(0x2A2D33),
    accent: hex(0x2D9CDB),
    scrollbar_thumb: hex(0x3A3E45),
    scrollbar_thumb_hover: hex(0x565C66),

    capacity_track: hex(0x2E323A),
    capacity_fill: hex(0x1580C4),
    capacity_full: hex(0xD9683A),
};

const LIGHT: Palette = Palette {
    window_bg: hex(0xEDEEF1),
    sidebar_bg: hex(0xF3F4F6),
    pane_bg: hex(0xFFFFFF),
    tab_bar_bg: hex(0xE7E9ED),
    toolbar_bg: hex(0xF7F8FA),
    header_bg: hex(0xF7F8FA),
    footer_bg: hex(0xF3F4F6),
    field_bg: hex(0xFFFFFF),

    row_hover: hex(0xE8F0FA),
    selection: hex(0x1580C4),
    selection_inactive: hex(0xDDE1E6),
    cursor_outline: hex(0x0E5F94),

    text: hex(0x14161A),
    text_muted: hex(0x596270),
    text_faint: hex(0x848C97),
    text_on_selection: hex(0xFFFFFF),

    tab_active: hex(0xFFFFFF),
    tab_hover: hex(0xF0F1F4),
    divider: hex(0xD3D7DD),
    accent: hex(0x1580C4),
    scrollbar_thumb: hex(0xB6BBC3),
    scrollbar_thumb_hover: hex(0x8D949E),

    capacity_track: hex(0xD8DCE2),
    capacity_fill: hex(0x1580C4),
    capacity_full: hex(0xC2552A),
};

/// Indigo, and darker than Dark. Contrasts are checked by the tests below,
/// like every other theme here.
const MIDNIGHT: Palette = Palette {
    window_bg: hex(0x0B0D16),
    sidebar_bg: hex(0x11141F),
    pane_bg: hex(0x171B29),
    tab_bar_bg: hex(0x11141F),
    toolbar_bg: hex(0x141828),
    header_bg: hex(0x141828),
    footer_bg: hex(0x11141F),
    field_bg: hex(0x0B0D16),

    row_hover: hex(0x222840),
    selection: hex(0x4C5FD7),
    selection_inactive: hex(0x2A3048),
    cursor_outline: hex(0x8A9BFF),

    text: hex(0xE6E9F5),
    text_muted: hex(0xA6AEC9),
    text_faint: hex(0x737B96),
    text_on_selection: hex(0xFFFFFF),

    tab_active: hex(0x171B29),
    tab_hover: hex(0x1B2032),
    divider: hex(0x2A3048),
    accent: hex(0x7A8CFF),
    scrollbar_thumb: hex(0x3A4160),
    scrollbar_thumb_hover: hex(0x566083),

    capacity_track: hex(0x2A3048),
    capacity_fill: hex(0x4C5FD7),
    capacity_full: hex(0xD9683A),
};

/// Paper rather than screen. The warm greys are what a light theme is for if
/// white is what you are trying to get away from.
const SEPIA: Palette = Palette {
    window_bg: hex(0xEDE6D8),
    sidebar_bg: hex(0xF3EDE1),
    pane_bg: hex(0xFBF6EC),
    tab_bar_bg: hex(0xE8E0D0),
    toolbar_bg: hex(0xF6F1E6),
    header_bg: hex(0xF6F1E6),
    footer_bg: hex(0xF3EDE1),
    field_bg: hex(0xFFFCF5),

    row_hover: hex(0xEDE3CE),
    selection: hex(0x8A5A12),
    selection_inactive: hex(0xE2D8C4),
    cursor_outline: hex(0x7A4E10),

    text: hex(0x2A2318),
    text_muted: hex(0x5F5645),
    text_faint: hex(0x8A8170),
    text_on_selection: hex(0xFFFFFF),

    tab_active: hex(0xFBF6EC),
    tab_hover: hex(0xF1EADC),
    divider: hex(0xD9CFBB),
    accent: hex(0x8A5A12),
    scrollbar_thumb: hex(0xC3B8A2),
    scrollbar_thumb_hover: hex(0x9C9179),

    capacity_track: hex(0xDCD2BE),
    capacity_fill: hex(0x8A5A12),
    capacity_full: hex(0xB3491F),
};

impl Theme {
    pub const DARK: Theme = Theme(0);

    /// A theme by position, falling back to the default rather than panicking:
    /// the index can come from a settings file somebody edited.
    pub fn at(index: usize) -> Self {
        if index < THEMES.len() {
            Theme(index)
        } else {
            Theme::default()
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        THEMES
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name.trim()))
            .map(Theme)
    }

    fn def(self) -> &'static ThemeDef {
        &THEMES[self.0]
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn index(self) -> usize {
        self.0
    }

    pub fn palette(self) -> &'static Palette {
        &self.def().palette
    }

    pub fn is_dark(self) -> bool {
        self.def().dark
    }

    /// The other side. Ctrl+D is a light switch, not a tour of the table —
    /// so from any dark theme this is the first light one, and back again.
    pub fn toggled(self) -> Self {
        let want = !self.is_dark();
        THEMES
            .iter()
            .position(|t| t.dark == want)
            .map(Theme)
            .unwrap_or(self)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn colours_convert_to_the_bgr_order_gdi_wants() {
        // COLORREF is 0x00BBGGRR, which is the opposite of every other API
        // here; getting it backwards turns the accent blue into orange.
        assert_eq!(colorref((1.0, 0.0, 0.0)).0, 0x0000FF);
        assert_eq!(colorref((0.0, 1.0, 0.0)).0, 0x00FF00);
        assert_eq!(colorref((0.0, 0.0, 1.0)).0, 0xFF0000);
    }

    #[test]
    fn colour_channels_are_clamped_not_wrapped() {
        assert_eq!(colorref((2.0, -1.0, 0.5)).0, 0x008000FF);
    }

    use super::*;

    fn luminance(c: Rgb) -> f32 {
        // WCAG relative luminance.
        let ch = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.0) + 0.7152 * ch(c.1) + 0.0722 * ch(c.2)
    }

    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn every_theme() -> impl Iterator<Item = Theme> {
        (0..THEMES.len()).map(Theme::at)
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(0xFFFFFF), (1.0, 1.0, 1.0));
        assert_eq!(hex(0x000000), (0.0, 0.0, 0.0));
        let (r, g, b) = hex(0x1580C4);
        assert!((r - 21.0 / 255.0).abs() < 1e-6);
        assert!((g - 128.0 / 255.0).abs() < 1e-6);
        assert!((b - 196.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn body_text_is_comfortably_readable() {
        for t in every_theme() {
            let p = t.palette();
            assert!(
                contrast(p.text, p.pane_bg) >= 7.0,
                "{:?} body text only {:.1}:1",
                t,
                contrast(p.text, p.pane_bg)
            );
        }
    }

    #[test]
    fn muted_column_text_clears_wcag_aa() {
        // This is the improvement over the look we are chasing: secondary
        // columns there sit around 4:1 and get hard to read.
        for t in every_theme() {
            let p = t.palette();
            let c = contrast(p.text_muted, p.pane_bg);
            assert!(c >= 4.5, "{:?} muted text only {:.1}:1", t, c);
        }
    }

    #[test]
    fn selected_row_text_is_readable() {
        for t in every_theme() {
            let p = t.palette();
            let c = contrast(p.text_on_selection, p.selection);
            assert!(c >= 4.0, "{:?} selection text only {:.1}:1", t, c);
        }
    }

    #[test]
    fn surfaces_step_in_a_visible_order() {
        // Each surface must be perceptibly distinct from the next, or the
        // panels blur into one flat sheet. Light themes climb too: the window
        // behind is the darkest thing in them.
        for t in every_theme() {
            let p = t.palette();
            let steps = [p.window_bg, p.sidebar_bg, p.pane_bg];
            for pair in steps.windows(2) {
                let d = luminance(pair[1]) - luminance(pair[0]);
                assert!(d > 0.0005, "{} surfaces too close: {:?}", t.name(), pair[0]);
            }
        }
    }

    #[test]
    fn a_theme_is_reachable_by_name_and_never_by_a_bad_index() {
        for t in every_theme() {
            assert_eq!(Theme::by_name(t.name()), Some(t));
            // Settings files are text, and text gets typed.
            assert_eq!(Theme::by_name(&t.name().to_uppercase()), Some(t));
        }
        assert_eq!(Theme::by_name("Chartreuse"), None);
        assert_eq!(Theme::at(THEMES.len()), Theme::default());
    }

    #[test]
    fn toggling_crosses_to_the_other_side_and_back() {
        for t in every_theme() {
            let other = t.toggled();
            assert_ne!(other.is_dark(), t.is_dark(), "{} did not cross", t.name());
            // And back to a theme on this side, which for Dark and Light is
            // the one you started from.
            assert_eq!(other.toggled().is_dark(), t.is_dark());
        }
        assert_eq!(Theme::DARK.toggled().name(), "Light");
        assert_eq!(Theme::DARK.toggled().toggled(), Theme::DARK);
    }

    #[test]
    fn hover_is_visible_against_the_row_background() {
        // The old palette had a 1/255 difference here, which showed as nothing.
        for t in every_theme() {
            let p = t.palette();
            let d = (luminance(p.row_hover) - luminance(p.pane_bg)).abs();
            assert!(d > 0.002, "{:?} hover is invisible ({:.5})", t, d);
        }
    }
}
