//! Plugin versions and version requirement constraints.
//!
//! Versions are semantic (`MAJOR.MINOR.PATCH`). Requirement strings support
//! the practical subset used by manifests: exact versions, wildcard padding
//! (`1.2`, `1`), and comparison prefixes (`>=`, `<=`, `>`, `<`, `=`).

use std::fmt;
use std::str::FromStr;

use crate::error::{PluginError, PluginResult};

/// A semantic plugin version.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PluginVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// A malformed version or version requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidVersion(pub String);

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid plugin version: {}", self.0)
    }
}

impl PluginVersion {
    /// The current plugin API version of the Nexus platform. Plugins refusing
    /// this major value cannot be activated.
    pub const API_VERSION: u32 = 1;

    /// Builds a version exactly.
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Zero version placeholder.
    pub const ZERO: Self = Self::new(0, 0, 0);

    /// Whether a manifest's declared API version works on this platform.
    /// Major must match exactly; this platform's API version is a simple
    /// integer counter (`1`).
    #[must_use]
    pub const fn api_compatible(manifest_api_version: u32) -> bool {
        manifest_api_version == Self::API_VERSION
    }

    /// Parses `1.2.3` (`1.2` and `1` pad with zero).
    ///
    /// # Errors
    /// [`InvalidVersion`] on malformed input.
    pub fn parse(candidate: &str) -> PluginResult<Self> {
        parse_version(candidate)
            .ok_or_else(|| PluginError::internal(format!("invalid version `{candidate}`")))
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for PluginVersion {
    type Err = InvalidVersion;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_version(s).ok_or_else(|| InvalidVersion(s.to_string()))
    }
}

fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub(crate) fn parse_version(candidate: &str) -> Option<PluginVersion> {
    let parts: Vec<&str> = candidate.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let major = parse_number(parts[0])?;
    let minor = match parts.get(1) {
        None => 0,
        Some(p) => parse_number(p)?,
    };
    let patch = match parts.get(2) {
        None => 0,
        Some(p) => parse_number(p)?,
    };
    Some(PluginVersion {
        major,
        minor,
        patch,
    })
}

/// A version requirement from a plugin manifest dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    /// No requirement: any version is accepted.
    Any,
    /// At least this version (`>= X.Y.Z`).
    AtLeast(PluginVersion),
    /// At most this version (`<= X.Y.Z`).
    UpTo(PluginVersion),
    /// Strictly greater (`> X.Y.Z`).
    Greater(PluginVersion),
    /// Strictly less (`< X.Y.Z`).
    Less(PluginVersion),
    /// Exact version (`= X.Y.Z`).
    Exact(PluginVersion),
    /// Wildcard padding: candidate major/minor must match exactly
    /// (`1.2` accepts 1.2.*, `1` accepts 1.*.*).
    Padded(PluginVersion),
}

impl VersionReq {
    /// Whether a concrete version satisfies this requirement.
    #[must_use]
    pub fn matches(&self, candidate: PluginVersion) -> bool {
        match self {
            Self::Any => true,
            Self::AtLeast(base) => candidate >= *base,
            Self::UpTo(ceil) => candidate <= *ceil,
            Self::Greater(base) => candidate > *base,
            Self::Less(ceil) => candidate < *ceil,
            Self::Exact(target) => candidate == *target,
            Self::Padded(pad) => {
                candidate.major == pad.major && (pad.minor == 0 || candidate.minor == pad.minor)
            }
        }
    }

    /// Parses a requirement string: `""`, `"regex"`-free comparisons or a
    /// bare version.
    ///
    /// # Errors
    /// [`PluginError::internal`] on malformed input.
    pub fn parse(candidate: &str) -> PluginResult<Self> {
        let trimmed = candidate.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("any") || trimmed == "*" {
            return Ok(Self::Any);
        }
        if let Some(rest) = trimmed.strip_prefix(">=") {
            return Ok(Self::AtLeast(parse_version(rest.trim()).ok_or_else(
                || PluginError::internal(format!("invalid version requirement `{candidate}`")),
            )?));
        }
        if let Some(rest) = trimmed.strip_prefix("<=") {
            return Ok(Self::UpTo(parse_version(rest.trim()).ok_or_else(|| {
                PluginError::internal(format!("invalid version requirement `{candidate}`"))
            })?));
        }
        if let Some(rest) = trimmed.strip_prefix(">") {
            return Ok(Self::Greater(parse_version(rest.trim()).ok_or_else(
                || PluginError::internal(format!("invalid version requirement `{candidate}`")),
            )?));
        }
        if let Some(rest) = trimmed.strip_prefix("<") {
            return Ok(Self::Less(parse_version(rest.trim()).ok_or_else(|| {
                PluginError::internal(format!("invalid version requirement `{candidate}`"))
            })?));
        }
        if let Some(rest) = trimmed.strip_prefix("=") {
            return Ok(Self::Exact(parse_version(rest.trim()).ok_or_else(
                || PluginError::internal(format!("invalid version requirement `{candidate}`")),
            )?));
        }
        let version = parse_version(trimmed).ok_or_else(|| {
            PluginError::internal(format!("invalid version requirement `{candidate}`"))
        })?;
        Ok(Self::Padded(version))
    }
}

impl FromStr for VersionReq {
    type Err = InvalidVersion;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).map_err(|_| InvalidVersion(s.to_string()))
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::AtLeast(v) => write!(f, ">={v}"),
            Self::UpTo(v) => write!(f, "<={v}"),
            Self::Greater(v) => write!(f, ">{v}"),
            Self::Less(v) => write!(f, "<{v}"),
            Self::Exact(v) => write!(f, "={v}"),
            Self::Padded(v) => write!(f, "{v}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_with_padding() {
        assert_eq!(
            PluginVersion::parse("1.2.3").unwrap(),
            PluginVersion::new(1, 2, 3)
        );
        assert_eq!(
            PluginVersion::parse("1.2").unwrap(),
            PluginVersion::new(1, 2, 0)
        );
        assert_eq!(
            PluginVersion::parse("1").unwrap(),
            PluginVersion::new(1, 0, 0)
        );
        assert!(PluginVersion::parse("1.x").is_err());
        assert!(PluginVersion::parse("..").is_err());
        assert!(PluginVersion::parse("99.99.99.99").is_err());
    }

    #[test]
    fn version_ordering() {
        let a = PluginVersion::new(1, 2, 0);
        let b = PluginVersion::new(1, 2, 0);
        let c = PluginVersion::new(1, 3, 0);
        assert_eq!(a, b);
        assert!(c > a);
    }

    #[test]
    fn requirements_match() {
        let v = PluginVersion::new(1, 4, 2);
        assert!(VersionReq::parse(">= 1.2").unwrap().matches(v));
        assert!(!VersionReq::parse(">= 1.5").unwrap().matches(v));
        assert!(VersionReq::parse("<2").unwrap().matches(v));
        assert!(VersionReq::parse("= 1.4.2").unwrap().matches(v));
        assert!(!VersionReq::parse("= 1.4.1").unwrap().matches(v));
        assert!(VersionReq::parse("1.4").unwrap().matches(v), "padded req");
        assert!(!VersionReq::parse("1.5").unwrap().matches(v));
        assert!(VersionReq::parse("1").unwrap().matches(v), "major pad");
        assert!(VersionReq::parse("*").unwrap().matches(v));
        assert!(VersionReq::parse("").unwrap().matches(v));
        assert!(VersionReq::parse("> 1.4.1").unwrap().matches(v));
        assert!(VersionReq::parse("<= 1.4.2").unwrap().matches(v));
        assert!(!VersionReq::parse("< 1.4.2").unwrap().matches(v));
    }

    #[test]
    fn display_round_trip() {
        let req = VersionReq::parse(">= 1.2").unwrap();
        assert_eq!(req.to_string(), ">=1.2.0");
    }
}
