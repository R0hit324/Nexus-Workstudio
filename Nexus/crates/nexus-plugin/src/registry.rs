//! The synchronized plugin registry: id -> manifest, state, permissions,
//! configuration. All mutation goes through checked state transitions.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use crate::config::{ConfigSchema, ConfigValue};
use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::lifecycle::PluginState;
use crate::manifest::PluginManifest;
use crate::permission::GrantedPermissions;

/// Platform trust reflected in executable-capability decisions (spec 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrust {
    /// Runs in-process, full native execution.
    Trusted,
    /// Limited subset, still in-process.
    Restricted,
    /// Must run isolated (WASM/process isolation; not in-process).
    Untrusted,
}

/// One registry row.
#[derive(Debug, Clone)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    pub state: PluginState,
    /// Granted permissions (subset of manifest request).
    pub granted: GrantedPermissions,
    /// Validated plugin configuration.
    pub configuration: BTreeMap<String, ConfigValue>,
    /// Declared configuration schema, if any.
    pub configuration_schema: Option<ConfigSchema>,
    /// Manager-assigned trust level.
    pub trust: PluginTrust,
    /// Last failure message.
    pub last_error: Option<String>,
    /// Loading scope.
    pub scope: crate::context::PluginScope,
}

/// Registry bookkeeping: reads/writes guarded, transitions validated.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    records: RwLock<BTreeMap<PluginId, PluginRecord>>,
}

impl PluginRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a discovered/validated record; rejects duplicates unless
    /// `replace` is set (re-discovery/update flows).
    ///
    /// # Errors
    /// Duplicate id without `replace`.
    pub fn upsert(&self, record: PluginRecord, replace: bool) -> PluginResult<()> {
        let mut records = self.records.write().expect("registry lock");
        if let Some(existing) = records.get(&record.manifest.id) {
            if !replace && existing.state != PluginState::Unloaded {
                return Err(PluginError::new(
                    Some(record.manifest.id.clone()),
                    "registry.upsert",
                    "plugin already registered and loaded",
                ));
            }
        }
        records.insert(record.manifest.id.clone(), record);
        Ok(())
    }

    /// A record clone.
    #[must_use]
    pub fn record(&self, id: &PluginId) -> Option<PluginRecord> {
        self.records.read().expect("registry lock").get(id).cloned()
    }

    /// Moves a plugin to the next state with transition validation.
    ///
    /// # Errors
    /// Unknown id or illegal transition.
    pub fn transition(&self, id: &PluginId, next: PluginState) -> PluginResult<()> {
        let mut records = self.records.write().expect("registry lock");
        let record = records.get_mut(id).ok_or_else(|| {
            PluginError::new(
                Some(id.clone()),
                "registry.transition",
                "plugin not registered",
            )
        })?;
        if !record.state.can_transition_to(next) {
            let from = record.state;
            record.state = PluginState::Failed;
            record.last_error = Some(format!("illegal lifecycle transition {from:?} -> {next:?}"));
            return Err(PluginError::new(
                Some(id.clone()),
                "registry.transition",
                format!("illegal lifecycle transition {from:?} -> {next:?}"),
            ));
        }
        record.state = next;
        Ok(())
    }

    /// Marks a plugin failed with the underlying reason.
    pub fn mark_failed(&self, id: &PluginId, reason: String) {
        let mut records = self.records.write().expect("registry lock");
        if let Some(record) = records.get_mut(id) {
            record.state = PluginState::Failed;
            record.last_error = Some(reason);
        }
    }

    /// Replaces granted permissions (approval flow).
    pub fn grant(&self, id: &PluginId, granted: GrantedPermissions) {
        let mut records = self.records.write().expect("registry lock");
        if let Some(record) = records.get_mut(id) {
            record.granted = granted;
        }
    }

    /// Replaces validated configuration.
    pub fn set_configuration(
        &self,
        id: &PluginId,
        configuration: BTreeMap<String, ConfigValue>,
    ) -> PluginResult<()> {
        let mut records = self.records.write().expect("registry lock");
        records
            .get_mut(id)
            .ok_or_else(|| {
                PluginError::new(
                    Some(id.clone()),
                    "configuration.set",
                    "plugin not registered",
                )
            })
            .map(|record| {
                record.configuration = configuration;
            })
    }

    /// Ids active in one state.
    #[must_use]
    pub fn ids_in_state(&self, state: PluginState) -> BTreeSet<PluginId> {
        self.records
            .read()
            .expect("registry lock")
            .values()
            .filter(|r| r.state == state)
            .map(|r| r.manifest.id.clone())
            .collect()
    }

    /// All records (status listing for administrators, spec 75).
    #[must_use]
    pub fn all(&self) -> Vec<PluginRecord> {
        self.records
            .read()
            .expect("registry lock")
            .values()
            .cloned()
            .collect()
    }

    /// Removes a record entirely (uninstall, spec 69).
    pub fn remove(&self, id: &PluginId) -> Option<PluginRecord> {
        self.records.write().expect("registry lock").remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PluginPermission;

    fn manifest(id: &str) -> PluginManifest {
        PluginManifest::parse_toml(&format!(
            "id = \"{id}\"\nname = \"t\"\nversion = \"1.0.0\"\nauthor = \"t\"\ndescription = \"t\"\n"
        ))
        .unwrap()
    }

    fn record(id: &str) -> PluginRecord {
        let m = manifest(id);
        PluginRecord {
            manifest: m,
            state: PluginState::Discovered,
            granted: GrantedPermissions::grant(
                &[PluginPermission::CubeRead],
                [PluginPermission::CubeRead],
            ),
            configuration: BTreeMap::new(),
            configuration_schema: None,
            trust: PluginTrust::Restricted,
            last_error: None,
            scope: crate::context::PluginScope::Server,
        }
    }

    #[test]
    fn upsert_and_transition_record() {
        let registry = PluginRegistry::new();
        let id = PluginId::new("org.nexus.a.alpha").unwrap();
        registry.upsert(record("org.nexus.a.alpha"), false).unwrap();

        registry.transition(&id, PluginState::Validated).unwrap();
        registry.transition(&id, PluginState::Loaded).unwrap();
        assert_eq!(registry.record(&id).unwrap().state, PluginState::Loaded);
        registry
            .upsert(record("org.nexus.a.alpha"), false)
            .unwrap_err();
        registry.upsert(record("org.nexus.a.alpha"), true).unwrap();
    }

    #[test]
    fn illegal_transition_marks_failed() {
        let registry = PluginRegistry::new();
        let id = PluginId::new("org.nexus.a.bravo").unwrap();
        registry.upsert(record("org.nexus.a.bravo"), false).unwrap();
        let err = registry.transition(&id, PluginState::Enabled).unwrap_err();
        assert!(err.message.contains("illegal lifecycle transition"));
        assert_eq!(registry.record(&id).unwrap().state, PluginState::Failed);
    }

    #[test]
    fn grants_replaced_by_id() {
        let registry = PluginRegistry::new();
        registry
            .upsert(record("org.nexus.a.charlie"), false)
            .unwrap();
        let id = PluginId::new("org.nexus.a.charlie").unwrap();
        registry.grant(
            &id,
            GrantedPermissions::from_set([PluginPermission::Network]),
        );
        assert!(registry
            .record(&id)
            .unwrap()
            .granted
            .allows(PluginPermission::Network));
    }
}
