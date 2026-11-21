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
}
