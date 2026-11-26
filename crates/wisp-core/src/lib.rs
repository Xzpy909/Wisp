//! Platform-neutral logic for Wisp.
//!
//! Nothing in this crate touches Windows APIs, so it builds and tests on any OS.

pub mod placement;
pub mod settings;

pub use placement::{popup_rect, Point, Rect};
pub use settings::{PopupStyle, Settings};
