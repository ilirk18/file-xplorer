// Color tokens for the dark and light palettes.
//
// Every colour the renderer draws comes from here so the two palettes stay in
// step. Values are linear-ish sRGB floats in 0..1, which is what Direct2D wants.

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

pub type Rgb = (f32, f32, f32);

/// One resolved palette. Field order matches roughly how things are painted:
/// chrome, then surfaces, then rows, then text, then accents.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window_bg: Rgb,
    pub sidebar_bg: Rgb,
    pub pane_bg: Rgb,
    pub tab_bar_bg: Rgb,
    pub header_bg: Rgb,
    pub status_bg: Rgb,

    pub row_alt: Rgb,
    pub row_hover: Rgb,
    pub selection: Rgb,
    /// Selection in the pane that does not have focus.
    pub selection_inactive: Rgb,
    pub cursor_outline: Rgb,

    pub text: Rgb,
    pub text_muted: Rgb,
    pub text_on_selection: Rgb,

    pub tab_active: Rgb,
    pub tab_hover: Rgb,
    pub divider: Rgb,
    pub focus: Rgb,
    pub scrollbar_thumb: Rgb,
    pub scrollbar_thumb_hover: Rgb,
}

const DARK: Palette = Palette {
    window_bg: (0.071, 0.071, 0.078),
    sidebar_bg: (0.086, 0.086, 0.094),
    pane_bg: (0.110, 0.110, 0.122),
    tab_bar_bg: (0.071, 0.071, 0.078),
    header_bg: (0.086, 0.086, 0.098),
    status_bg: (0.086, 0.086, 0.094),

    // Striping is a genuine step, not the 1/255 the old palette used.
    row_alt: (0.137, 0.137, 0.153),
    row_hover: (0.180, 0.184, 0.208),
    selection: (0.157, 0.325, 0.576),
    selection_inactive: (0.220, 0.235, 0.267),
    cursor_outline: (0.478, 0.596, 0.804),

    text: (0.918, 0.925, 0.945),
    text_muted: (0.545, 0.561, 0.612),
    text_on_selection: (1.0, 1.0, 1.0),

    tab_active: (0.157, 0.161, 0.188),
    tab_hover: (0.129, 0.133, 0.153),
    divider: (0.208, 0.212, 0.239),
    focus: (0.376, 0.518, 0.780),
    scrollbar_thumb: (0.290, 0.298, 0.337),
    scrollbar_thumb_hover: (0.404, 0.416, 0.463),
};

const LIGHT: Palette = Palette {
    window_bg: (0.945, 0.945, 0.957),
    sidebar_bg: (0.925, 0.929, 0.941),
    pane_bg: (1.0, 1.0, 1.0),
    tab_bar_bg: (0.914, 0.918, 0.933),
    header_bg: (0.965, 0.965, 0.973),
    status_bg: (0.925, 0.929, 0.941),

    row_alt: (0.965, 0.969, 0.976),
    row_hover: (0.902, 0.925, 0.965),
    selection: (0.796, 0.867, 0.984),
    selection_inactive: (0.882, 0.886, 0.898),
    cursor_outline: (0.235, 0.435, 0.706),

    text: (0.106, 0.114, 0.133),
    text_muted: (0.404, 0.420, 0.463),
    text_on_selection: (0.055, 0.075, 0.110),

    tab_active: (1.0, 1.0, 1.0),
    tab_hover: (0.949, 0.953, 0.965),
    divider: (0.804, 0.812, 0.831),
    focus: (0.235, 0.435, 0.706),
    scrollbar_thumb: (0.706, 0.714, 0.737),
    scrollbar_thumb_hover: (0.573, 0.584, 0.612),
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
