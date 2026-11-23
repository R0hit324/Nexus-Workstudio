//! Persistent local preferences (recent cubes, layout, keymap, theme).
//!
//! Small JSON file, bounded in size. Loading merges defaults so an older or
//! partial file never makes the UI unusable.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::commands::Command;
use super::input::Key;
use super::keymap::{Binding, Mode};

/// How line numbers are shown in the editor gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LineNumbersMode {
    Absolute,
    #[default]
    Relative,
    None,
}

/// Editor-specific preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorPrefs {
    #[serde(default)]
    pub line_numbers: LineNumbersMode,
    #[serde(default = "default_tab_width")]
    pub tab_width: u8,
    #[serde(default)]
    pub soft_wrap: bool,
    #[serde(default)]
    pub show_whitespace: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            line_numbers: LineNumbersMode::Relative,
            tab_width: default_tab_width(),
            soft_wrap: false,
            show_whitespace: false,
        }
    }
}

fn default_tab_width() -> u8 {
    4
}

/// Full persisted preference set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalPreferences {
    /// Most recently opened cube names, newest first (bounded).
    #[serde(default)]
    pub recent_cubes: Vec<String>,
    /// Which panels are visible by name.
    #[serde(default)]
    pub visible_panels: Vec<String>,
    /// User-customized keybindings.
    #[serde(default)]
    pub bindings: Vec<BindingSpec>,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub editor: EditorPrefs,
    /// (columns, rows) of the last window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<(u16, u16)>,
}

impl Default for LocalPreferences {
    fn default() -> Self {
        Self {
            recent_cubes: Vec::new(),
            visible_panels: Vec::new(),
            bindings: Vec::new(),
            theme: "nexus-dark".to_owned(),
            editor: EditorPrefs::default(),
            window: None,
        }
    }
}

/// A serializable keybinding: a key string plus a command name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BindingSpec {
    pub key: String,
    pub command: String,
}

impl LocalPreferences {
    /// Maximum recent-cube entries retained.
    pub const MAX_RECENT: usize = 16;

    /// Pushes a cube onto the recent list (dedup + bound).
    pub fn remember_cube(&mut self, name: &str) {
        self.recent_cubes.retain(|c| c != name);
        self.recent_cubes.insert(0, name.to_owned());
        self.recent_cubes.truncate(Self::MAX_RECENT);
    }

    /// The default preferences file path.
    #[must_use]
    pub fn default_path() -> PathBuf {
        use std::env;
        let base = env::var_os("APPDATA")
            .or_else(|| env::var_os("XDG_CONFIG_HOME"))
            .or_else(|| env::var_os("HOME"))
            .or_else(|| Some(std::env::temp_dir().into_os_string()))
            .unwrap();
        PathBuf::from(base).join("nexus").join("prefs.json")
    }

    /// Loads preferences from `path`, falling back to defaults.
    #[must_use]
    pub fn load(path: &std::path::Path) -> Self {
        let fallback = Self::default();
        match std::fs::read_to_string(path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or(fallback),
            Err(_) => fallback,
        }
    }

    /// Saves preferences, creating parent directories as needed.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).expect("preferences serialize");
        std::fs::write(path, json)
    }

    /// Converts persisted bindings into [`Binding`]s, skipping invalid ones.
    #[must_use]
    pub fn to_bindings(&self) -> Vec<Binding> {
        self.bindings
            .iter()
            .filter_map(|spec| {
                let key = Key::parse(&spec.key)?;
                let command = Command::from_str(&spec.command)?;
                Some(Binding::new(Mode::Normal, key, command))
            })
            .collect()
    }
}

/// A panel id string used in `visible_panels`.
#[must_use]
pub fn panel_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0 => "explorer",
        1 => "editor",
        2 => "terminal",
        3 => "users",
        4 => "chat",
        5 => "build",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_cubes_dedup_and_bound() {
        let mut p = LocalPreferences::default();
        p.remember_cube("alpha");
        p.remember_cube("beta");
        p.remember_cube("alpha");
        assert_eq!(p.recent_cubes, vec!["alpha", "beta"]);
        for i in 0..(LocalPreferences::MAX_RECENT + 5) {
            p.remember_cube(&format!("c{i}"));
        }
        assert_eq!(p.recent_cubes.len(), LocalPreferences::MAX_RECENT);
    }

    #[test]
    fn load_falls_back_to_defaults() {
        let missing = std::env::temp_dir().join("nexus-does-not-exist.json");
        let p = LocalPreferences::load(&missing);
        assert_eq!(p.theme, "nexus-dark");
        assert!(p.recent_cubes.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let path =
            std::env::temp_dir().join(format!("nexus-prefs-test-{}.json", std::process::id()));
        let mut p = LocalPreferences::default();
        p.remember_cube("cube");
        p.editor.soft_wrap = true;
        p.bindings.push(BindingSpec {
            key: "Ctrl-P".into(),
            command: "command-palette".into(),
        });
        p.save(&path).unwrap();
        let loaded = LocalPreferences::load(&path);
        assert_eq!(loaded.recent_cubes, vec!["cube"]);
        assert!(loaded.editor.soft_wrap);
        assert_eq!(loaded.to_bindings().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bindings_skip_invalid() {
        let p = LocalPreferences {
            bindings: vec![
                BindingSpec {
                    key: "Ctrl-P".into(),
                    command: "command-palette".into(),
                },
                BindingSpec {
                    key: "garbage-key".into(),
                    command: "command-palette".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(p.to_bindings().len(), 1);
    }
}
