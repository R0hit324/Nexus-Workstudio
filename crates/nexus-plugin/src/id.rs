//! Stable, globally unique plugin identity.
//!
//! A `PluginId` is a reverse-DNS style, dot-separated, lowercase identifier
//! such as `org.nexus.git` or `community.foo.theme`. It is stable across
//! plugin versions and must never be derived from a display name.

use std::fmt;
use std::str::FromStr;

/// Unique, stable identifier of a plugin.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

/// Validation error for a malformed plugin id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPluginId(pub String);

impl fmt::Display for InvalidPluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid plugin id: {}", self.0)
    }
}

impl PluginId {
    /// The plugin API version namespace of Nexus core itself.
    pub const VENDOR: &'static str = "org.nexus";

    /// Creates a plugin id from a candidate string, validating the shape.
    ///
    /// # Errors
    /// Returns [`InvalidPluginId`] when the id is not at least three
    /// reverse-DNS segments of lowercase ASCII alphanumerics/hyphens.
    pub fn new(candidate: impl Into<String>) -> Result<Self, InvalidPluginId> {
        let candidate = candidate.into();
        validate(&candidate)?;
        Ok(Self(candidate))
    }

    /// Creates an id without validation. Only for reading parsed manifests
    /// that were already validated.
    #[must_use]
    pub fn from_validated(value: String) -> Self {
        Self(value)
    }

    /// The vendor namespace (first segment).
    #[must_use]
    pub fn vendor(&self) -> &str {
        self.0.split('.').next().unwrap_or("")
    }

    /// The last segment, i.e. the short plugin name within its namespace.
    #[must_use]
    pub fn short_name(&self) -> &str {
        self.0.rsplit('.').next().unwrap_or("")
    }

    /// The raw canonical text of the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate(candidate: &str) -> Result<(), InvalidPluginId> {
    let segments: Vec<&str> = candidate.split('.').collect();
    if segments.len() < 3 {
        return Err(InvalidPluginId(candidate.to_string()));
    }
    for segment in segments {
        if segment.is_empty() {
            return Err(InvalidPluginId(candidate.to_string()));
        }
        for ch in segment.chars() {
            let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
            if !ok {
                return Err(InvalidPluginId(candidate.to_string()));
            }
        }
    }
    Ok(())
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PluginId {
    type Error = InvalidPluginId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PluginId> for String {
    fn from(value: PluginId) -> Self {
        value.0
    }
}

impl FromStr for PluginId {
    type Err = InvalidPluginId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        for id in ["org.nexus.git", "community.foo.theme", "a.b.c-1.d"] {
            assert!(PluginId::new(id).is_ok(), "{id} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        for id in [
            "",
            "git",
            "org.nexus",
            "Org.Nexus.Git",
            "org.nexus.git!",
            "org..git",
        ] {
            assert!(PluginId::new(id).is_err(), "{id} should be invalid");
        }
    }

    #[test]
    fn vendor_and_short_name() {
        let id = PluginId::new("org.nexus.git").unwrap();
        assert_eq!(id.vendor(), "org");
        assert_eq!(id.short_name(), "git");
    }

    #[test]
    fn round_trips_through_string() {
        let id = PluginId::new("org.nexus.rust").unwrap();
        let as_string: String = id.clone().into();
        assert_eq!(PluginId::try_from(as_string).unwrap(), id);
    }
}
