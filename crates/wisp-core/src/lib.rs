//! Platform-neutral logic for Wisp.
//!
//! Nothing in this crate touches Windows APIs, so it builds and tests on any OS.

pub mod backup;
pub mod clip;
pub mod images;
pub mod placement;
pub mod search;
pub mod settings;
pub mod store;
pub mod update;

pub use clip::{Captured, Clip, ClipContent, Kind};
pub use placement::{popup_rect, Point, Rect};
pub use search::Filter;
pub use settings::{PopupStyle, Settings};
pub use store::Store;
