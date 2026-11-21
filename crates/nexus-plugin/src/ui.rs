//! UI extension registry: panels and keybindings declared by plugins.
//!
//! Plugins may register panels and keybindings only in their own id space,
//! within hard quotas (spec §92), and never over CORE-reserved ids. Dropping
//! a plugin's registrations happens atomically through
//! [`UiRegistry::unregister_plugin`] so disable/uninstall leaves no ghost UI.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;

/// Panel registrations allowed per plugin.
pub const MAX_PANELS_PER_PLUGIN: usize = 8;

/// Keybindings allowed per plugin.
pub const MAX_KEYBINDINGS_PER_PLUGIN: usize = 16;

/// Reserved board panel ids plugins must never take.
pub const RESERVED_PANEL_IDS: [&str; 6] = [
    "core.activity",
    "core.output",
    "core.statusline",
    "core.command-palette",
    "core.diagnostics",
    "core.terminal",
];

/// A plugin-provided panel definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDefinition {
    /// Fully qualified panel id (`git.branch-panel`).
    pub id: String,
    /// Display title.
    pub title: String,
    /// The owning plugin.
    pub owner: PluginId,
    /// Panel order hint for layout stacking.
    pub order: u16,
    /// Whether the panel can be hidden by the user.
    pub hideable: bool,
}

/// A plugin-provided keybinding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyBinding {
    /// Key combo string (`ctrl+shift+g`), normalized lowercase.
    pub key: String,
    /// Command executed (`git.status`).
    pub command_id: String,
    /// The binding owner.
    pub owner: PluginId,
}

#[derive(Default)]
struct RegistryState {
    panels: BTreeMap<String, PanelDefinition>,
    keybindings: BTreeMap<String, KeyBinding>,
    panel_counts: BTreeMap<PluginId, usize>,
    binding_counts: BTreeMap<PluginId, usize>,
}

/// The UI extension registry (host shared; TUI reads here).
#[derive(Default)]
pub struct UiRegistry {
    state: RwLock<RegistryState>,
}

fn combo_key(key: &str, owner: &PluginId) -> String {
    format!("{key}::{owner}")
}

impl UiRegistry {
    /// Builds an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a plugin panel.
    ///
    /// # Errors
    /// Reserved id taken, unnamespaced id, duplicate id, or plugin quota.
    pub fn register_panel(&self, definition: PanelDefinition) -> PluginResult<()> {
        if RESERVED_PANEL_IDS.contains(&definition.id.as_str()) {
            return Err(PluginError::new(
                Some(definition.owner.clone()),
                "ui.panel",
                format!("panel id `{}` is reserved by the core UI", definition.id),
            ));
        }
        if definition.id.split('.').count() < 2 {
            return Err(PluginError::new(
                Some(definition.owner.clone()),
                "ui.panel",
                format!(
                    "panel id `{}` must be namespaced (e.g. `git.branch-panel`)",
                    definition.id
                ),
            ));
        }
        if definition.title.trim().is_empty() {
            return Err(PluginError::new(
                Some(definition.owner.clone()),
                "ui.panel",
                "panel title must not be empty",
            ));
        }

        let mut state = self.state.write().expect("ui registry lock");
        if state.panels.contains_key(&definition.id) {
            return Err(PluginError::new(
                Some(definition.owner.clone()),
                "ui.panel",
                format!("panel id `{}` is already registered", definition.id),
            ));
        }
        let count = state
            .panel_counts
            .entry(definition.owner.clone())
            .or_default();
        if *count >= MAX_PANELS_PER_PLUGIN {
            return Err(PluginError::new(
                Some(definition.owner.clone()),
                "ui.panel",
                format!("panel quota {} exceeded for plugin", MAX_PANELS_PER_PLUGIN),
            ));
        }
        *count += 1;
        state.panels.insert(definition.id.clone(), definition);
        Ok(())
    }

    /// Registers a plugin keybinding; combos are unique per owner.
    ///
    /// # Errors
    /// Reserved core combo, duplicate, empty key/command, or plugin quota.
    pub fn register_keybinding(&self, binding: KeyBinding) -> PluginResult<()> {
        if binding.key.trim().is_empty() || binding.command_id.trim().is_empty() {
            return Err(PluginError::new(
                Some(binding.owner.clone()),
                "ui.keybinding",
                "key and command id must not be empty",
            ));
        }
        if binding.key.starts_with("ctrl+c") || binding.key == "ctrl+q" {
            return Err(PluginError::new(
                Some(binding.owner.clone()),
                "ui.keybinding",
                "combo is reserved for the core terminal experience",
            ));
        }

        let normalized = binding.key.to_lowercase();
        let mut state = self.state.write().expect("ui registry lock");
        let id = combo_key(&normalized, &binding.owner);
        if state.keybindings.contains_key(&id) {
            return Err(PluginError::new(
                Some(binding.owner.clone()),
                "ui.keybinding",
                format!("combo `{}` already registered by this plugin", binding.key),
            ));
        }
        let count = state
            .binding_counts
            .entry(binding.owner.clone())
            .or_default();
        if *count >= MAX_KEYBINDINGS_PER_PLUGIN {
            return Err(PluginError::new(
                Some(binding.owner.clone()),
                "ui.keybinding",
                format!(
                    "keybinding quota {} exceeded for plugin",
                    MAX_KEYBINDINGS_PER_PLUGIN
                ),
            ));
        }
        *count += 1;
        state.keybindings.insert(
            id,
            KeyBinding {
                key: normalized,
                command_id: binding.command_id,
                owner: binding.owner,
            },
        );
        Ok(())
    }

    /// All panels, ordered by the plugin-declared hint.
    #[must_use]
    pub fn panels(&self) -> Vec<PanelDefinition> {
        let mut panels: Vec<PanelDefinition> = self
            .state
            .read()
            .expect("ui registry lock")
            .panels
            .values()
            .cloned()
            .collect();
        panels.sort_by_key(|panel| (panel.order, panel.id.clone()));
        panels
    }

    /// All keybindings owned by one plugin.
    #[must_use]
    pub fn keybindings_of(&self, owner: &PluginId) -> Vec<KeyBinding> {
        self.state
            .read()
            .expect("ui registry lock")
            .keybindings
            .values()
            .filter(|binding| binding.owner == *owner)
            .cloned()
            .collect()
    }

    /// The binding should trigger for `key` in the owner's command space.
    #[must_use]
    pub fn binding_for(&self, key: &str, owner: &PluginId) -> Option<KeyBinding> {
        let normalized = key.to_lowercase();
        self.state
            .read()
            .expect("ui registry lock")
            .keybindings
            .get(&combo_key(&normalized, owner))
            .cloned()
    }

    /// Removes everything a plugin registered (panels + keybindings).
    pub fn unregister_plugin(&self, owner: &PluginId) {
        let mut state = self.state.write().expect("ui registry lock");
        let doomed_panels: BTreeSet<String> = state
            .panels
            .iter()
            .filter(|(_, panel)| panel.owner == *owner)
            .map(|(id, _)| id.clone())
            .collect();
        for id in doomed_panels {
            state.panels.remove(&id);
        }
        let doomed_bindings: BTreeSet<String> = state
            .keybindings
            .iter()
            .filter(|(_, binding)| binding.owner == *owner)
            .map(|(id, _)| id.clone())
            .collect();
        for id in doomed_bindings {
            state.keybindings.remove(&id);
        }
        state.panel_counts.remove(owner);
        state.binding_counts.remove(owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(name: &str) -> PluginId {
        PluginId::new(name).unwrap()
    }

    fn panel(owner: &PluginId, id: &str) -> PanelDefinition {
        PanelDefinition {
            id: String::from(id),
            title: String::from("Branches"),
            owner: owner.clone(),
            order: 1,
            hideable: true,
        }
    }

    #[test]
    fn registers_and_lists() {
        let registry = UiRegistry::new();
        registry
            .register_panel(panel(&owner("org.nexus.git"), "git.branches"))
            .unwrap();
        registry
            .register_panel(panel(&owner("org.nexus.rust"), "rust.checks"))
            .unwrap();
        assert_eq!(registry.panels().len(), 2);
    }

    #[test]
    fn reserved_and_unnamespaced_rejected() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        let err = registry
            .register_panel(panel(&git, "core.output"))
            .unwrap_err();
        assert!(err.message.contains("reserved"));
        let err = registry
            .register_panel(panel(&git, "loose-panel"))
            .unwrap_err();
        assert!(err.message.contains("namespaced"));
        let err = registry
            .register_panel(PanelDefinition {
                id: String::from("git.empty"),
                title: String::from("   "),
                owner: git.clone(),
                order: 1,
                hideable: true,
            })
            .unwrap_err();
        assert!(err.message.contains("title"));
    }

    #[test]
    fn duplicate_panel_rejected() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        registry
            .register_panel(panel(&git, "git.branches"))
            .unwrap();
        assert!(registry
            .register_panel(panel(&git, "git.branches"))
            .is_err());
    }

    #[test]
    fn quota_is_enforced() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        for index in 0..MAX_PANELS_PER_PLUGIN {
            registry
                .register_panel(panel(&git, &format!("git.panel-{index}")))
                .unwrap();
        }
        let err = registry
            .register_panel(panel(&git, &format!("git.panel-{}", MAX_PANELS_PER_PLUGIN)))
            .unwrap_err();
        assert!(err.message.contains("quota"));
    }

    #[test]
    fn keybindings_namespaced_and_lookup() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        registry
            .register_keybinding(KeyBinding {
                key: String::from("Ctrl+Shift+G"),
                command_id: String::from("git.status"),
                owner: git.clone(),
            })
            .unwrap();
        assert!(registry.binding_for("ctrl+shift+g", &git).is_some());
        assert!(registry
            .binding_for("ctrl+shift+g", &owner("org.nexus.rust"))
            .is_none());
    }

    #[test]
    fn reserved_combos_blocked() {
        let registry = UiRegistry::new();
        for combo in ["ctrl+c", "ctrl+q"] {
            let err = registry
                .register_keybinding(KeyBinding {
                    key: String::from(combo),
                    command_id: String::from("git.status"),
                    owner: owner("org.nexus.git"),
                })
                .unwrap_err();
            assert!(err.message.contains("reserved"), "combo {combo}");
        }
    }

    #[test]
    fn keybinding_quota() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        for index in 0..MAX_KEYBINDINGS_PER_PLUGIN {
            registry
                .register_keybinding(KeyBinding {
                    key: format!("ctrl+alt+F{index}"),
                    command_id: String::from("git.status"),
                    owner: git.clone(),
                })
                .unwrap();
        }
        assert!(registry
            .register_keybinding(KeyBinding {
                key: String::from("ctrl+alt+zz"),
                command_id: String::from("git.status"),
                owner: git.clone(),
            })
            .is_err());
    }

    #[test]
    fn unregister_cleans_everything() {
        let registry = UiRegistry::new();
        let git = owner("org.nexus.git");
        registry
            .register_panel(panel(&git, "git.branches"))
            .unwrap();
        registry
            .register_keybinding(KeyBinding {
                key: String::from("ctrl+shift+g"),
                command_id: String::from("git.status"),
                owner: git.clone(),
            })
            .unwrap();
        registry.unregister_plugin(&git);
        assert!(registry.panels().is_empty());
        assert!(registry.keybindings_of(&git).is_empty());
        // Quota reset: plugin can register again after re-enable.
        registry
            .register_panel(panel(&git, "git.branches"))
            .unwrap();
        assert_eq!(registry.panels().len(), 1);
    }
}
