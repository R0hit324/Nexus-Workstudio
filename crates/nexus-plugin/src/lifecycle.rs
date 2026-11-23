//! Plugin lifecycle states.
//!
//! Workflow: an id is [`PluginState::Discovered`] when its manifest is found,
//! becomes [`PluginState::Validated`] after manifest/compatibility checks
//! pass, then [`PluginState::Loaded`] (registered in-process), moves to
//! [`PluginState::Initialized`] after `on_load` succeeds, and finally reaches
//! [`PluginState::Enabled`]. Unloading and failure are modeled explicitly.
//! All transitions are checked; anything else falls back to
//! [`PluginState::Failed`].

/// Lifecycle position of one plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    /// Manifest located, not yet validated.
    Discovered,
    /// Manifest valid and compatible; ready to load.
    Validated,
    /// Registered in the runtime (entry point materialized).
    Loaded,
    /// `on_load` completed with its context; `on_enable` allowed.
    Initialized,
    /// Fully active: commands/events/UI registrations live.
    Enabled,
    /// Disabled by configuration: registrations removed, hooks released.
    Disabled,
    /// Unloaded: no runtime presence, must be re-discovered/loaded.
    Unloaded,
    /// Failed validation, initialization, runtime error, or crash.
    Failed,
}

impl PluginState {
    /// Whether a direct transition `self -> next` is legal per the workflow.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use PluginState as S;
        matches!(
            (self, next),
            (S::Discovered, S::Validated)
                | (S::Discovered, S::Failed)
                | (S::Discovered, S::Unloaded)
                | (S::Validated, S::Loaded)
                | (S::Validated, S::Failed)
                | (S::Validated, S::Unloaded)
                | (S::Loaded, S::Initialized)
                | (S::Loaded, S::Failed)
                | (S::Loaded, S::Unloaded)
                | (S::Initialized, S::Enabled)
                | (S::Initialized, S::Failed)
                | (S::Initialized, S::Unloaded)
                | (S::Enabled, S::Disabled)
                | (S::Enabled, S::Failed)
                | (S::Disabled, S::Initialized)
                | (S::Disabled, S::Unloaded)
                | (S::Initialized, S::Disabled)
        )
    }

    /// Whether registrations (commands, events, UI) are considered live.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether the plugin is present in the runtime but not active.
    #[must_use]
    pub const fn is_present(self) -> bool {
        matches!(
            self,
            Self::Loaded | Self::Initialized | Self::Enabled | Self::Disabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_transitions() {
        assert!(PluginState::Discovered.can_transition_to(PluginState::Validated));
        assert!(PluginState::Validated.can_transition_to(PluginState::Loaded));
        assert!(PluginState::Loaded.can_transition_to(PluginState::Initialized));
        assert!(PluginState::Initialized.can_transition_to(PluginState::Enabled));
        assert!(PluginState::Enabled.can_transition_to(PluginState::Disabled));
        assert!(PluginState::Disabled.can_transition_to(PluginState::Unloaded));
    }

    #[test]
    fn illegal_transitions_rejected() {
        assert!(!PluginState::Discovered.can_transition_to(PluginState::Enabled));
        assert!(!PluginState::Validated.can_transition_to(PluginState::Enabled));
        assert!(!PluginState::Failed.can_transition_to(PluginState::Enabled));
        assert!(!PluginState::Unloaded.can_transition_to(PluginState::Loaded));
        assert!(!PluginState::Enabled.can_transition_to(PluginState::Discovered));
    }

    #[test]
    fn re_enable_via_reinitialization() {
        assert!(PluginState::Disabled.can_transition_to(PluginState::Initialized));
        assert!(PluginState::Loaded.can_transition_to(PluginState::Failed));
        assert!(PluginState::Initialized.can_transition_to(PluginState::Disabled));
    }

    #[test]
    fn activation_queries() {
        assert!(PluginState::Enabled.is_active());
        assert!(!PluginState::Initialized.is_active());
        assert!(PluginState::Enabled.is_present());
        assert!(PluginState::Disabled.is_present());
        assert!(!PluginState::Discovered.is_present());
        assert!(!PluginState::Unloaded.is_present());
    }
}
