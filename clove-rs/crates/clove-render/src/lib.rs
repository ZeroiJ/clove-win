//! Clove's visual identity as pure, framework-independent CPU rendering.
//!
//! Nothing in this crate knows about a UI toolkit: it produces straight-alpha RGBA that
//! any framework can upload (Rune `Pixels`, GPUI images, a PNG in CI).

pub mod bar;
pub mod tokens;

pub use bar::{BarRenderer, BarSpec, Segment};
pub use tokens::Rgb;
