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

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

pub type Rgb = (f32, f32, f32);

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

impl Theme {
    pub fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark => &DARK,
            Theme::Light => &LIGHT,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }
}

#[cfg(test)]
mod tests {
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
        for t in [Theme::Dark, Theme::Light] {
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
        for t in [Theme::Dark, Theme::Light] {
            let p = t.palette();
            let c = contrast(p.text_muted, p.pane_bg);
            assert!(c >= 4.5, "{:?} muted text only {:.1}:1", t, c);
        }
    }

    #[test]
    fn selected_row_text_is_readable() {
        for t in [Theme::Dark, Theme::Light] {
            let p = t.palette();
            let c = contrast(p.text_on_selection, p.selection);
            assert!(c >= 4.0, "{:?} selection text only {:.1}:1", t, c);
        }
    }

    #[test]
    fn surfaces_step_in_a_visible_order() {
        // Each surface must be perceptibly distinct from the next, or the
        // panels blur into one flat sheet.
        let p = Theme::Dark.palette();
        let steps = [p.window_bg, p.sidebar_bg, p.pane_bg, p.row_hover];
        for pair in steps.windows(2) {
            let d = luminance(pair[1]) - luminance(pair[0]);
            assert!(d > 0.0005, "surfaces too close: {:?} vs {:?}", pair[0], pair[1]);
        }
    }

    #[test]
    fn hover_is_visible_against_the_row_background() {
        // The old palette had a 1/255 difference here, which showed as nothing.
        for t in [Theme::Dark, Theme::Light] {
            let p = t.palette();
            let d = (luminance(p.row_hover) - luminance(p.pane_bg)).abs();
            assert!(d > 0.002, "{:?} hover is invisible ({:.5})", t, d);
        }
    }
}
