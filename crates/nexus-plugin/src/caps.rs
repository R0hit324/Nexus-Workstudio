//! Capability sub-traits: the typed surfaces plugins specialize into.
//!
//! The plugin capability taxonomy lives in [`crate::capability`]; this module
//! gives each family a stable data-only definition surface plus contextual
//! traits. The platform consumes these definitions; keeping them plain data
//! means host wiring can change without breaking plugin contracts.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::plugin::Plugin;

// ---- language --------------------------------------------------------------

/// How a language plugin describes a language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageDefinition {
    /// Canonical language id (`rust`, `python`).
    pub id: String,
    /// Display name (`Rust`).
    pub name: String,
    /// Extensions without dot (`rs`, `py`).
    pub file_extensions: Vec<String>,
    /// Line-comment prefix (empty when the language uses block comments).
    pub comment_prefix: String,
    /// Default build command when a build plugin does not override it.
    pub build_command: String,
    /// Default test command.
    pub test_command: String,
}

impl LanguageDefinition {
    /// Whether a file path belongs to this language (extension match).
    #[must_use]
    pub fn matches_file(&self, path: &PathBuf) -> bool {
        let extension = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        !extension.is_empty()
            && self
                .file_extensions
                .iter()
                .any(|called| called == &extension)
    }
}

/// A plugin contributing language support.
#[async_trait]
pub trait LanguagePlugin: Plugin {
    /// This language's definition (stable across versions).
    fn language_definition(&self) -> LanguageDefinition;
}

// ---- build -----------------------------------------------------------------

/// One build system a build plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSystemDefinition {
    /// Capability name (`cargo`, `npm`).
    pub name: String,
    /// How a workspace is detected (`Cargo.toml` present).
    pub marker_file: String,
    /// Build command.
    pub build_command: String,
    /// Test command.
    pub test_command: String,
    /// Check command (fast diagnostics; may equal build).
    pub check_command: String,
}

/// A plugin contributing build-system support.
#[async_trait]
pub trait BuildPlugin: Plugin {
    /// Build systems this plugin knows (usually exactly one).
    fn build_systems(&self) -> Vec<BuildSystemDefinition>;
}

// ---- runtime ---------------------------------------------------------------

/// One runtime backend a runtime plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBackendDefinition {
    /// Backend id (`local`, `docker`, `wsl`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether the backend requires per-use permission confirmation.
    pub needs_confirmation: bool,
    /// Requires the docker binary (host wiring enforces).
    pub requires_binary: Option<String>,
}

/// A plugin contributing runtime backends.
#[async_trait]
pub trait RuntimePlugin: Plugin {
    /// Backends contributed (may be empty when unavailable).
    fn runtime_backends(&self) -> Vec<RuntimeBackendDefinition>;
}

// ---- debugger --------------------------------------------------------------

/// A source breakpoint definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    /// File path.
    pub file: PathBuf,
    /// 1-based line number.
    pub line: u32,
    /// Optional condition expression.
    pub condition: Option<String>,
}

/// How a debugger plugin declares itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebuggerDefinition {
    /// Debugger id (`lldb`, `gdb`).
    pub id: String,
    /// Default launch binary.
    pub binary: String,
    /// Adapter command prefix.
    pub adapter_command: String,
}

/// A plugin contributing debugger support.
#[async_trait]
pub trait DebuggerPlugin: Plugin {
    /// This debugger's definition.
    fn debugger_definition(&self) -> DebuggerDefinition;
}

// ---- theme -----------------------------------------------------------------

/// Canonical palette keys every theme must define.
pub const THEME_KEYS: [&str; 8] = [
    "background",
    "foreground",
    "accent",
    "selection",
    "error",
    "warning",
    "statusline",
    "inactive",
];

/// One named theme a theme plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDefinition {
    /// Theme id (`dark-slate`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// `THEME_KEYS` → hex or named colors. Extra keys allowed.
    pub palette: std::collections::BTreeMap<String, String>,
}

impl ThemeDefinition {
    /// Validation: all canonical keys present and non-empty.
    ///
    /// # Errors
    /// A missing or empty canonical key.
    pub fn validate(&self) -> crate::PluginResult<()> {
        for key in THEME_KEYS {
            let missing = self
                .palette
                .get(key)
                .map(std::string::String::is_empty)
                .unwrap_or(true);
            if missing {
                return Err(crate::PluginError::new(
                    None,
                    "ui.theme",
                    format!("theme `{}` is missing color key `{key}`", self.id),
                ));
            }
        }
        Ok(())
    }
}

/// A plugin contributing themes.
#[async_trait]
pub trait ThemePlugin: Plugin {
    /// Themes contributed (each must `validate()`).
    fn themes(&self) -> Vec<ThemeDefinition>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::id::PluginId;

    struct RustLangPlugin;

    #[async_trait]
    impl Plugin for RustLangPlugin {
        fn id(&self) -> PluginId {
            PluginId::new("org.nexus.rust").unwrap()
        }
        async fn on_load(&self, _: &crate::context::PluginContext) -> crate::PluginResult<()> {
            Ok(())
        }
        async fn on_enable(&self, _: &crate::context::PluginContext) -> crate::PluginResult<()> {
            Ok(())
        }
        async fn on_disable(&self, _: &crate::context::PluginContext) -> crate::PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &crate::context::PluginContext) -> crate::PluginResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl LanguagePlugin for RustLangPlugin {
        fn language_definition(&self) -> LanguageDefinition {
            LanguageDefinition {
                id: String::from("rust"),
                name: String::from("Rust"),
                file_extensions: vec![String::from("rs")],
                comment_prefix: String::from("//"),
                build_command: String::from("cargo build"),
                test_command: String::from("cargo test"),
            }
        }
    }

    #[async_trait]
    impl BuildPlugin for RustLangPlugin {
        fn build_systems(&self) -> Vec<BuildSystemDefinition> {
            vec![BuildSystemDefinition {
                name: String::from("cargo"),
                marker_file: String::from("Cargo.toml"),
                build_command: String::from("cargo build"),
                test_command: String::from("cargo test"),
                check_command: String::from("cargo check"),
            }]
        }
    }

    #[test]
    fn language_matches_file_extensions_case_insensitive() {
        let plugin = RustLangPlugin;
        let definition = plugin.language_definition();
        assert!(definition.matches_file(&PathBuf::from("src/lib.rs")));
        assert!(definition.matches_file(&PathBuf::from("SRC/Main.RS")));
        assert!(!definition.matches_file(&PathBuf::from("src/lib.txt")));
    }

    #[test]
    fn build_system_declares_cargo() {
        let systems = RustLangPlugin.build_systems();
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0].name, "cargo");
        assert_eq!(systems[0].marker_file, "Cargo.toml");
    }

    #[test]
    fn theme_validation_checks_canonical_keys() {
        let mut palette = BTreeMap::new();
        for key in THEME_KEYS {
            palette.insert(String::from(key), String::from("#000000"));
        }
        let mut theme = ThemeDefinition {
            id: String::from("dark-slate"),
            name: String::from("Dark Slate"),
            palette,
        };
        theme.validate().unwrap();
        theme.palette.remove("error");
        assert!(theme.validate().is_err(), "missing key must fail");
        theme.palette.insert(String::from("error"), String::new());
        assert!(theme.validate().is_err(), "empty key must fail");
    }
}
