#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const SURFACE: Self = Self::rgb(31, 32, 31);
    pub const PANEL: Self = Self::rgb(239, 237, 230);
    pub const PANEL_ALT: Self = Self::rgb(219, 217, 209);
    pub const BORDER: Self = Self::rgb(87, 88, 84);
    pub const TEXT: Self = Self::rgb(30, 31, 30);
    pub const TEXT_INVERTED: Self = Self::rgb(244, 242, 235);
    pub const TEXT_MUTED: Self = Self::rgb(135, 135, 128);
    pub const ACCENT: Self = Self::rgb(214, 167, 82);
    pub const ACCENT_DARK: Self = Self::rgb(142, 103, 45);
    pub const GLASS: Self = Self::rgb(50, 52, 50);
    pub const WINDOW_DARK: Self = Self::rgb(20, 21, 20);
    pub const WARNING: Self = Self::rgb(225, 180, 93);
    pub const ERROR: Self = Self::rgb(218, 104, 94);
    pub const SUCCESS: Self = Self::rgb(94, 176, 126);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}
