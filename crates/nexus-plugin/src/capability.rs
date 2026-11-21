//! Plugin category capabilities.
//!
//! A plugin may implement multiple capabilities; each capability maps to a
//! stable extension point the platform exposes (language support, build
//! integration, UI, and so on).

/// A functional capability a plugin can provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    /// Syntax highlighting, compiler adapters, formatters, linters.
    Language,
    /// Runtime backend implementations (Docker, Podman, local process, ...).
    Runtime,
    /// Build systems, compilers, task runners, diagnostics parsers.
    Build,
    /// Debugging sessions (launch, attach, breakpoints, stepping).
    Debugger,
    /// Version control integration (git, ...).
    Vcs,
    /// Colors, styles, icons; never application logic.
    Theme,
    /// Panel/tab/status/dialog registration.
    Ui,
    /// `:`-prefixed user commands.
    Command,
    /// AI provider integration (full architecture: Part 12).
    Ai,
    /// Catch-all for future integration surfaces.
    Integration,
}

impl PluginCapability {
    /// The canonical snake_case name, also the manifest token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::Runtime => "runtime",
            Self::Build => "build",
            Self::Debugger => "debugger",
            Self::Vcs => "vcs",
            Self::Theme => "theme",
            Self::Ui => "ui",
            Self::Command => "command",
            Self::Ai => "ai",
            Self::Integration => "integration",
        }
    }

    /// A short human description used by listings.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Language => "Language support and toolchain adapters",
            Self::Runtime => "Runtime backend implementations",
            Self::Build => "Build systems and compilers",
            Self::Debugger => "Debugging sessions",
            Self::Vcs => "Version control integration",
            Self::Theme => "Visual themes",
            Self::Ui => "UI extension points",
            Self::Command => "User commands",
            Self::Ai => "AI provider integration",
            Self::Integration => "Future integrations",
        }
    }

    /// Capabilities that imply UI-visible registration and therefore require
    /// the `ui.register` permission.
    #[must_use]
    pub const fn requires_ui_register(self) -> bool {
        matches!(self, Self::Ui | Self::Theme | Self::Command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_snake_case_tokens() {
        assert_eq!(PluginCapability::Language.as_str(), "language");
        assert_eq!(PluginCapability::Vcs.as_str(), "vcs");
        assert_eq!(PluginCapability::Ai.as_str(), "ai");
    }

    #[test]
    fn ui_visible_capabilities_require_register() {
        assert!(PluginCapability::Ui.requires_ui_register());
        assert!(PluginCapability::Theme.requires_ui_register());
        assert!(PluginCapability::Command.requires_ui_register());
        assert!(!PluginCapability::Language.requires_ui_register());
        assert!(!PluginCapability::Runtime.requires_ui_register());
    }
}
