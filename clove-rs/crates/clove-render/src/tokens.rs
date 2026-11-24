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

/// Palette (Vesper dark / Omarchy).
pub const BG: Rgb = Rgb(0x10, 0x11, 0x14);
pub const SIDEBAR_BG: Rgb = Rgb(0x15, 0x16, 0x19);
pub const SURFACE: Rgb = Rgb(0x1E, 0x20, 0x24);
pub const SURFACE_HOVER: Rgb = Rgb(0x27, 0x29, 0x2E);
pub const SURFACE_ACTIVE: Rgb = Rgb(0x31, 0x33, 0x3A);
pub const INK: Rgb = Rgb(0xFA, 0xFA, 0xFA);
pub const INK_SECONDARY: Rgb = Rgb(0x9E, 0xA1, 0xA8);
pub const INK_MUTED: Rgb = Rgb(0x5A, 0x5D, 0x66);
pub const HAIRLINE: Rgb = Rgb(0x22, 0x24, 0x28);
pub const ACCENT: Rgb = Rgb(0xFF, 0xC7, 0x99); // Vesper peach/accent
pub const ACCENT_MUTED: Rgb = Rgb(0x3A, 0x2F, 0x24);
pub const FOLDER_ORANGE: Rgb = Rgb(0xE9, 0x54, 0x20); // Yaru terracotta orange
pub const SUCCESS: Rgb = Rgb(0x99, 0xFF, 0xE4); // Vesper mint
pub const WARNING: Rgb = Rgb(0xFF, 0xC7, 0x99); // Vesper orange/yellow
pub const DANGER: Rgb = Rgb(0xFF, 0x80, 0x80);  // Vesper red

/// Spacing scale (points), from `Metrics.swift`.
pub const SPACING: [f32; 5] = [4.0, 8.0, 12.0, 16.0, 24.0];

/// Corner radii (points), from `Metrics.swift`.
pub const RADIUS_CARD: f32 = 12.0;
pub const RADIUS_CONTROL: f32 = 6.0;
pub const RADIUS_ROW: f32 = 6.0;
pub const RADIUS_BAR: f32 = 8.0;
pub const RADIUS_PILL: f32 = 999.0;

/// Typography scale (size pt, weight), from `Typography.swift`.
pub const TYPE_BODY: (f32, f32) = (13.0, 400.0);
pub const TYPE_LABEL: (f32, f32) = (12.0, 500.0);
pub const TYPE_TITLE: (f32, f32) = (17.0, 600.0);

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
