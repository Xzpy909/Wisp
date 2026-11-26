//! User settings, stored as JSON in `%APPDATA%\Wisp\settings.json`.
//!
//! Milestone 2 moves these into the SQLite database; the JSON file keeps the
//! skeleton free of a database dependency.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PopupStyle {
    /// Command bar in the upper third of the screen.
    #[default]
    BeamCentre,
    /// Command bar next to the text caret (or the mouse pointer).
    BeamCursor,
    /// Tall panel against the taskbar edge, near the tray.
    Drawer,
}

impl PopupStyle {
    pub const ALL: [PopupStyle; 3] = [Self::BeamCentre, Self::BeamCursor, Self::Drawer];

    pub fn label(self) -> &'static str {
        match self {
            Self::BeamCentre => "Beam, centre of screen",
            Self::BeamCursor => "Beam, at cursor",
            Self::Drawer => "Drawer, by the tray",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    pub popup_style: PopupStyle,
    /// Global shortcut, e.g. `Alt+V` or `Ctrl+Shift+Space`.
    pub hotkey: String,
    pub start_with_windows: bool,
    /// Acrylic glass behind the popup. Needs the GPU renderer, which costs
    /// about 35 MB more memory, so it is off by default.
    pub glass_effect: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            popup_style: PopupStyle::default(),
            hotkey: "Alt+V".into(),
            start_with_windows: true,
            glass_effect: false,
        }
    }
}

impl Settings {
    /// `%APPDATA%\Wisp` on Windows; `$XDG_CONFIG_HOME/wisp` or `~/.config/wisp` elsewhere.
    pub fn dir() -> PathBuf {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("Wisp");
        }
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("wisp");
        }
        let home = std::env::var_os("HOME").unwrap_or_default();
        PathBuf::from(home).join(".config").join("wisp")
    }

    pub fn default_path() -> PathBuf {
        Self::dir().join("settings.json")
    }

    /// Load settings. Returns `(settings, first_run)`; a missing or unreadable
    /// file yields defaults, and `first_run` is true only when it is missing.
    pub fn load(path: &Path) -> (Self, bool) {
        match std::fs::read_to_string(path) {
            Ok(text) => (serde_json::from_str(&text).unwrap_or_default(), false),
            Err(_) => (Self::default(), true),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).expect("settings serialise");
        // Write then rename so a crash never leaves a half-written file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let dir = std::env::temp_dir().join(format!("wisp-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        let s = Settings {
            popup_style: PopupStyle::Drawer,
            hotkey: "Ctrl+Shift+Space".into(),
            start_with_windows: false,
            glass_effect: true,
        };
        s.save(&path).unwrap();
        let (loaded, first) = Settings::load(&path);
        assert_eq!(loaded, s);
        assert!(!first);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_file_is_first_run_with_defaults() {
        let (s, first) = Settings::load(Path::new("/definitely/not/here/settings.json"));
        assert!(first);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"popup-style":"beam-cursor","future":1}"#).unwrap();
        assert_eq!(s.popup_style, PopupStyle::BeamCursor);
        assert_eq!(s.hotkey, "Alt+V");
        assert!(s.start_with_windows);
        assert!(!s.glass_effect);
    }
}
