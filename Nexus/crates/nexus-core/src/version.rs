use serde::{Deserialize, Serialize};

/// An explicit version number attached to key protocols and persistent
/// structures. Never assume the latest client equals the latest server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Version(u32);

impl Version {
    /// Constructs a version from a raw number.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the underlying numeric value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Current version of the wire protocol.
pub const PROTOCOL_VERSION: Version = Version::new(1);

/// Current version of the workspace model.
pub const WORKSPACE_VERSION: Version = Version::new(1);

/// Current version of the event schema.
pub const EVENT_SCHEMA_VERSION: Version = Version::new(1);

/// Current version of the snapshot format.
pub const SNAPSHOT_VERSION: Version = Version::new(1);

/// Current version of the plugin API.
pub const PLUGIN_API_VERSION: Version = Version::new(1);

/// A single, ordered migration step toward a newer schema version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    /// The schema version this step migrates data to.
    pub version: u32,
    /// Human-readable summary of what the step does.
    pub description: &'static str,
}

/// Registry of ordered schema migrations.
///
/// Deployments register migration steps as new schema versions land; the
/// registry then reports the forward-only upgrade path from any installed
/// version. Migration is deliberately forward-only: a downgrade step is not
/// representable and must go through the documented rollback procedure
/// instead (Part 15, sections 9-11 and critical deployment rule 3).
#[derive(Debug, Default)]
pub struct MigrationRegistry {
    steps: Vec<MigrationStep>,
}

impl MigrationRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a forward migration step.
    ///
    /// # Errors
    /// Returns an error when the step version is not strictly greater than the
    /// last registered step.
    pub fn register(
        &mut self,
        version: u32,
        description: &'static str,
    ) -> std::result::Result<(), String> {
        let last = self.steps.last().map_or(0, |step| step.version);
        if version <= last {
            return Err(format!(
                "migration version {version} must be greater than {last}"
            ));
        }
        self.steps.push(MigrationStep {
            version,
            description,
        });
        Ok(())
    }

    /// Returns the next migration step after `from_version`, if any.
    #[must_use]
    pub fn next(&self, from_version: u32) -> Option<MigrationStep> {
        self.steps
            .iter()
            .find(|step| step.version > from_version)
            .cloned()
    }

    /// The highest registered migration version.
    #[must_use]
    pub fn latest(&self) -> u32 {
        self.steps.last().map_or(0, |step| step.version)
    }

    /// The registered steps in registration order.
    #[must_use]
    pub fn steps(&self) -> &[MigrationStep] {
        &self.steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_and_serialize() {
        assert!(Version::new(1) < Version::new(2));
        assert_eq!(PROTOCOL_VERSION.get(), 1);
        let json = serde_json::to_string(&PROTOCOL_VERSION).unwrap();
        let decoded: Version = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, PROTOCOL_VERSION);
    }

    #[test]
    fn display_is_the_number() {
        assert_eq!(Version::new(7).to_string(), "7");
    }

    #[test]
    fn migration_registry_starts_empty() {
        let registry = MigrationRegistry::new();
        assert!(registry.steps().is_empty());
        assert_eq!(registry.latest(), 0);
        assert!(registry.next(0).is_none());
    }

    #[test]
    fn next_returns_the_forward_plan_from_any_older_version() {
        let mut registry = MigrationRegistry::new();
        registry.register(2, "add event payload hash").unwrap();
        registry.register(3, "add snapshot checksum").unwrap();
        assert_eq!(registry.next(0).unwrap().version, 2);
        assert_eq!(registry.next(1).unwrap().version, 2);
        assert_eq!(registry.next(2).unwrap().version, 3);
    }

    #[test]
    fn next_returns_none_at_or_above_latest() {
        let mut registry = MigrationRegistry::new();
        registry.register(2, "step").unwrap();
        assert!(registry.next(2).is_none());
        assert!(registry.next(99).is_none());
    }

    #[test]
    fn register_rejects_non_increasing_versions() {
        let mut registry = MigrationRegistry::new();
        registry.register(2, "first").unwrap();
        assert!(registry.register(2, "duplicate").is_err());
        assert!(registry.register(1, "downgrade").is_err());
        assert_eq!(registry.steps().len(), 1);
    }

    #[test]
    fn register_accepts_strictly_increasing_versions() {
        let mut registry = MigrationRegistry::new();
        registry.register(2, "first").unwrap();
        registry.register(3, "second").unwrap();
        assert_eq!(registry.latest(), 3);
        assert_eq!(registry.steps().len(), 2);
    }
}
