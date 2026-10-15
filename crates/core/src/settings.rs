//! User settings, persisted as JSON (SPEC §7).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::privacy::{DEFAULT_IGNORED_APPS, PrivacyFilter};
use crate::queue::QueueDirection;
use crate::retention::Retention;
use crate::shortcut::{ModifierKey, Shortcut};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub activation_shortcut: Shortcut,
    pub paste_queue_shortcut: Shortcut,
    pub quick_paste_modifier: ModifierKey,
    pub plain_text_modifier: ModifierKey,
    /// Paste into the frontmost app (true) or only copy to the clipboard.
    pub direct_paste: bool,
    pub always_plain_text: bool,
    pub sound_effects: bool,
    pub show_in_menu_bar: bool,
    pub keep_history: Retention,
    pub ignore_confidential: bool,
    pub ignore_transient: bool,
    pub show_during_screen_sharing: bool,
    pub ignored_apps: Vec<String>,
    pub ignored_types: Vec<String>,
    pub paste_queue_direction: QueueDirection,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            activation_shortcut: Shortcut::new("v", true, true, false, false),
            paste_queue_shortcut: Shortcut::new("c", true, true, false, false),
            quick_paste_modifier: ModifierKey::Cmd,
            plain_text_modifier: ModifierKey::Shift,
            direct_paste: true,
            always_plain_text: false,
            sound_effects: false,
            show_in_menu_bar: true,
            keep_history: Retention::Month,
            ignore_confidential: true,
            ignore_transient: true,
            show_during_screen_sharing: false,
            ignored_apps: DEFAULT_IGNORED_APPS.iter().map(|s| s.to_string()).collect(),
            ignored_types: Vec::new(),
            paste_queue_direction: QueueDirection::Fifo,
        }
    }
}

impl Settings {
    /// Loads settings; a missing file yields defaults. A corrupt file is moved
    /// aside to `settings.json.bad` so the user's edits aren't silently lost.
    pub fn load(path: &Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Settings::default();
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => settings.sanitized(),
            Err(_) => {
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                Settings::default()
            }
        }
    }

    /// Writes atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context("creating settings directory")?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).context("writing settings")?;
        std::fs::rename(&tmp, path).context("replacing settings")?;
        Ok(())
    }

    pub fn sanitized(mut self) -> Self {
        if self.quick_paste_modifier == self.plain_text_modifier {
            self.plain_text_modifier = if self.quick_paste_modifier == ModifierKey::Shift {
                ModifierKey::Alt
            } else {
                ModifierKey::Shift
            };
        }
        self.ignored_apps.retain(|a| !a.trim().is_empty());
        self.ignored_apps.dedup();
        self
    }

    pub fn privacy_filter(&self) -> PrivacyFilter {
        PrivacyFilter {
            ignore_confidential: self.ignore_confidential,
            ignore_transient: self.ignore_transient,
            ignored_types: self.ignored_types.clone(),
            ignored_apps: self.ignored_apps.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());

        let settings = Settings {
            keep_history: Retention::Week,
            activation_shortcut: "ctrl+alt+p".parse().unwrap(),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
    }

    #[test]
    fn partial_files_fill_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Unknown keys (e.g. from older versions) are ignored.
        std::fs::write(&path, r#"{"sound_effects": true, "panel_height": 9000}"#).unwrap();
        let settings = Settings::load(&path);
        assert!(settings.sound_effects);
        assert_eq!(settings.keep_history, Retention::Month);
    }

    #[test]
    fn corrupt_files_are_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
        assert!(!path.exists());
    }

    #[test]
    fn conflicting_modifiers_are_fixed() {
        let s = Settings {
            plain_text_modifier: ModifierKey::Cmd,
            ..Settings::default()
        };
        assert_eq!(s.sanitized().plain_text_modifier, ModifierKey::Shift);
        let s = Settings {
            quick_paste_modifier: ModifierKey::Shift,
            plain_text_modifier: ModifierKey::Shift,
            ..Settings::default()
        };
        assert_eq!(s.sanitized().plain_text_modifier, ModifierKey::Alt);
    }
}
