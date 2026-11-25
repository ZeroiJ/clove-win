//! Design tokens ported from `Clove/Design/Palette.swift`, `Typography.swift`, and
//! `Metrics.swift`. Values are the dark appearance (Clove's default) as sRGB.

/// An sRGB colour, 8 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb(r, g, b)
    }

    pub const fn to_array(self) -> [u8; 3] {
        [self.0, self.1, self.2]
    }

    pub const fn to_f32(self) -> [f32; 3] {
        [
            self.0 as f32 / 255.0,
            self.1 as f32 / 255.0,
            self.2 as f32 / 255.0,
        ]
    }
}

/// Palette (Libadwaita Dark / Omarchy glass).
pub const BG: Rgb = Rgb(0x1D, 0x1D, 0x20);
pub const SIDEBAR_BG: Rgb = Rgb(0x2E, 0x2E, 0x32);
pub const SURFACE: Rgb = Rgb(0x2E, 0x2E, 0x32);
pub const SURFACE_HOVER: Rgb = Rgb(0x3A, 0x3A, 0x40);
pub const SURFACE_ACTIVE: Rgb = Rgb(0x45, 0x45, 0x4C);
pub const INK: Rgb = Rgb(0xF3, 0xF3, 0xF5);
pub const INK_SECONDARY: Rgb = Rgb(0xB0, 0xB0, 0xB6);
pub const INK_MUTED: Rgb = Rgb(0x78, 0x78, 0x80);
pub const HAIRLINE: Rgb = Rgb(0x35, 0x35, 0x3C);
pub const ACCENT: Rgb = Rgb(0x35, 0x84, 0xE4); // Adwaita blue
pub const ACCENT_MUTED: Rgb = Rgb(0x25, 0x33, 0x46);
pub const FOLDER_ORANGE: Rgb = Rgb(0xE9, 0x54, 0x20); // Yaru terracotta orange
pub const SUCCESS: Rgb = Rgb(0x57, 0xE3, 0x89); // Adwaita green
pub const WARNING: Rgb = Rgb(0xF8, 0xE4, 0x5C); // Adwaita yellow
pub const DANGER: Rgb = Rgb(0xF6, 0x61, 0x51);  // Adwaita red

/// Spacing scale (points), from `Metrics.swift`.
pub const SPACING: [f32; 5] = [4.0, 8.0, 12.0, 16.0, 24.0];

/// Corner radii (points), from `Metrics.swift`.
pub const RADIUS_CARD: f32 = 12.0;
pub const RADIUS_CONTROL: f32 = 6.0;
pub const RADIUS_ROW: f32 = 6.0;
pub const RADIUS_PATHBAR: f32 = 9.0;
pub const RADIUS_BAR: f32 = 8.0;
pub const RADIUS_PILL: f32 = 999.0;

/// Typography scale (size pt, weight), from `Typography.swift`.
pub const TYPE_BODY: (f32, f32) = (13.5, 400.0);
pub const TYPE_LABEL: (f32, f32) = (12.0, 500.0);
pub const TYPE_TITLE: (f32, f32) = (15.0, 700.0);

/// `StorageKind` bar hues, dark mode, in enum order (`Clove/Storage/StorageKind.swift`).
pub const STORAGE_KIND_RGB: [[f32; 3]; 9] = [
    [0.349, 0.612, 1.000], // Apps
    [0.436, 0.855, 0.453], // Code
    [0.946, 0.404, 0.691], // Photos
    [0.639, 0.449, 1.000], // Movies
    [0.332, 0.826, 0.946], // Documents
    [0.969, 0.818, 0.265], // Music
    [0.698, 0.729, 0.784], // System
    [0.982, 0.558, 0.233], // Caches
    [0.941, 0.328, 0.284], // Trash
];

pub const STORAGE_KIND_NAMES: [&str; 9] = [
    "Apps", "Code", "Photos", "Movies", "Documents", "Music", "System", "Caches", "Trash",
];
