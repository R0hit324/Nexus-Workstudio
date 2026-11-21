//! Plugin manifests.
//!
//! Every plugin ships a TOML manifest (`plugin.toml`) that the platform
//! validates before any code runs. The manifest is the single source of
//! truth for identity, versioning, capabilities, permissions, dependencies,
//! and compatibility checks.

use std::path::{Path, PathBuf};

use crate::capability::PluginCapability;
use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::permission::PluginPermission;
use crate::version::{PluginVersion, VersionReq};

/// The manifest platform API version this build supports.
pub const SUPPORTED_API_VERSION: u32 = crate::version::PluginVersion::API_VERSION;

/// A parsed and structurally validated `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Stable identity.
    pub id: PluginId,
    /// Display name (never used as identity).
    pub name: String,
    /// Plugin's own semantic version.
    pub version: PluginVersion,
    /// Which platform plugin API the plugin wants.
    pub api_version: u32,
    /// Author or organization.
    pub author: String,
    /// Short description.
    pub description: String,
    /// Where the executable plugin payload lives (engine-dependent).
    pub entry_point: Option<String>,
    /// Declared capability set.
    pub capabilities: Vec<PluginCapability>,
    /// Declared permission set (subset of what is granted).
    pub permissions: Vec<PluginPermission>,
    /// Other plugins this one requires.
    pub dependencies: Vec<PluginDependency>,
    /// Optional configuration schema marker (dotted schema id).
    pub config_schema: Option<String>,
    /// Optional platform filter, e.g. `["linux", "windows"]`.
    pub platforms: Vec<String>,
    /// File this manifest came from (informational, not part of equality).
    pub source: Option<PathBuf>,
}

/// A single dependency requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDependency {
    pub id: PluginId,
    pub version_req: VersionReq,
}

impl PluginManifest {
    /// Parses a TOML manifest and validates it structurally.
    ///
    /// # Errors
    /// [`PluginError`] when the TOML is malformed or validation fails.
    pub fn parse_toml(text: &str) -> PluginResult<Self> {
        let raw: RawManifest = toml::from_str(text)
            .map_err(|e| PluginError::internal(format!("malformed manifest: {e}")))?;
        let manifest = raw.validate()?;
        Ok(manifest)
    }

    /// Reads and parses a `plugin.toml` file.
    ///
    /// # Errors
    /// I/O or parse errors.
    pub fn from_path(path: impl AsRef<Path>) -> PluginResult<Self> {
        let text = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            PluginError::internal(format!("cannot read {}: {e}", path.as_ref().display()))
        })?;
        let mut manifest = Self::parse_toml(&text)?;
        manifest.source = Some(path.as_ref().to_path_buf());
        Ok(manifest)
    }

    /// Full validation: identity, API version, capability/permission
    /// coherence, and dependency self-reference.
    ///
    /// Uses [`PluginError`] so callers can attach context uniformly.
    ///
    /// # Errors
    /// Validation failures listed above.
    pub fn validate(&self) -> PluginResult<()> {
        let id = self.id.clone();
        if self.name.trim().is_empty() {
            return Err(PluginError::new(
                Some(id),
                "manifest.validate",
                "`name` must not be empty",
            ));
        }
        if !PluginVersion::api_compatible(self.api_version) {
            return Err(PluginError::new(
                Some(id),
                "compatibility.check",
                format!(
                    "manifest api_version {} is incompatible with platform API {}",
                    self.api_version, SUPPORTED_API_VERSION
                ),
            ));
        }
        if self.dependencies.iter().any(|d| d.id == self.id) {
            return Err(PluginError::new(
                Some(id),
                "manifest.validate",
                "manifest declares itself as a dependency",
            ));
        }
        if self.capabilities.iter().any(|c| c.requires_ui_register())
            && !self.permissions.contains(&PluginPermission::UiRegister)
        {
            return Err(PluginError::new(
                Some(id),
                "manifest.validate",
                "Capabilities `ui`, `theme`, or `command` require the `ui.register` permission",
            ));
        }
        Ok(())
    }

    /// Permissions the platform needs approval for before this manifest can
    /// be enabled (declared + sensitive + not yet granted).
    #[must_use]
    pub fn approval_requirements(
        &self,
        granted: &crate::permission::GrantedPermissions,
    ) -> Vec<PluginPermission> {
        crate::permission::GrantedPermissions::missing_approvals(&self.permissions, granted)
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    api_version: Option<u32>,
    author: Option<String>,
    description: Option<String>,
    entry_point: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
    dependencies: Option<Vec<RawDependency>>,
    config_schema: Option<String>,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct RawDependency {
    id: String,
    version: Option<String>,
}

/// Validation Liaisons: parse and convert all raw fields.
impl RawManifest {
    fn validate(self) -> PluginResult<PluginManifest> {
        let missing = |field: &str| {
            PluginError::internal(format!("manifest missing required field `{field}`"))
        };
        let id = PluginId::new(self.id.clone().ok_or_else(|| missing("id"))?)?;
        let name = self.name.ok_or_else(|| missing("name"))?;
        let version = PluginVersion::parse(&self.version.ok_or_else(|| missing("version"))?)?;
        // `api_version` defaults to the current platform API when omitted.
        let api_version = self.api_version.unwrap_or(SUPPORTED_API_VERSION);
        let author = self.author.ok_or_else(|| missing("author"))?;
        let description = self.description.ok_or_else(|| missing("description"))?;

        let mut capabilities = Vec::new();
        for token in &self.capabilities {
            capabilities.push(parse_capability(token).ok_or_else(|| {
                PluginError::internal(format!("manifest unknown capability `{token}`"))
            })?);
        }
        let mut permissions = Vec::new();
        for token in &self.permissions {
            permissions.push(PluginPermission::parse(token).ok_or_else(|| {
                PluginError::internal(format!("manifest unknown permission `{token}`"))
            })?);
        }

        let mut dependencies = Vec::new();
        for raw in self.dependencies.unwrap_or_default() {
            let id = PluginId::new(raw.id)?;
            let version_req = match raw.version {
                Some(text) => VersionReq::parse(&text)
                    .map_err(|e| PluginError::internal(format!("dependency `{}`: {e}", id)))?,
                None => VersionReq::Any,
            };
            dependencies.push(PluginDependency { id, version_req });
        }

        let manifest = PluginManifest {
            id,
            name,
            version,
            api_version,
            author,
            description,
            entry_point: self.entry_point,
            capabilities,
            permissions,
            dependencies,
            config_schema: self.config_schema,
            platforms: self.platforms.unwrap_or_default(),
            source: None,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

fn parse_capability(token: &str) -> Option<PluginCapability> {
    use PluginCapability as C;
    let tokens = [
        ("language", C::Language),
        ("runtime", C::Runtime),
        ("build", C::Build),
        ("debugger", C::Debugger),
        ("vcs", C::Vcs),
        ("theme", C::Theme),
        ("ui", C::Ui),
        ("command", C::Command),
        ("ai", C::Ai),
        ("integration", C::Integration),
    ];
    tokens
        .iter()
        .find(|(needle, _)| *needle == token)
        .map(|(_, capability)| *capability)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id = "org.nexus.git"
name = "Git Integration"
version = "1.0.0"
api_version = 1
author = "Nexus"
description = "Git integration for Nexus"
capabilities = ["vcs", "command"]
permissions = ["filesystem.read", "process.execute", "cube.read", "ui.register"]
"#;

    #[test]
    fn parses_the_documented_sample() {
        let manifest = PluginManifest::parse_toml(SAMPLE).unwrap();
        assert_eq!(manifest.id.as_str(), "org.nexus.git");
        assert_eq!(manifest.version.to_string(), "1.0.0");
        assert_eq!(manifest.api_version, 1);
        assert_eq!(
            manifest.capabilities,
            vec![PluginCapability::Vcs, PluginCapability::Command]
        );
        assert_eq!(manifest.permissions.len(), 4);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn api_version_defaults_when_omitted() {
        let manifest = PluginManifest::parse_toml(SAMPLE)
            .map(|m| m.api_version)
            .map(|v| v.to_string());
        assert!(manifest.is_ok());
    }

    #[test]
    fn rejects_missing_fields() {
        assert!(PluginManifest::parse_toml("id = \"org.nexus.a.b\"").is_err());
    }

    #[test]
    fn ui_capability_requires_ui_register() {
        let text = r#"
id = "community.foo.ui"
name = "Foo UI"
version = "0.1.0"
author = "Foo"
description = "Panel demo"
capabilities = ["ui"]
permissions = []
"#;
        let err = PluginManifest::parse_toml(text).unwrap_err();
        assert!(err.message.contains("ui.register"));
    }

    #[test]
    fn incompatible_api_version_rejected() {
        let text = SAMPLE.replace("api_version = 1", "api_version = 99");
        let err = PluginManifest::parse_toml(&text).unwrap_err();
        assert!(err.message.contains("api_version"));
    }

    #[test]
    fn self_dependency_rejected() {
        let text = format!(
            r#"
id = "org.nexus.git"
name = "Git"
version = "1.0.0"
author = "n"
description = "d"

[[dependencies]]
id = "org.nexus.git"
"#
        );
        let err = PluginManifest::parse_toml(&text).unwrap_err();
        assert!(err.message.contains("itself"));
    }

    #[test]
    fn parses_dependencies_with_requirements() {
        let text = r#"
id = "org.nexus.debugger"
name = "Debugger"
version = "0.1.0"
author = "n"
description = "d"

[[dependencies]]
id = "org.nexus.rust"
version = ">= 1.2"
"#;
        let manifest = PluginManifest::parse_toml(text).unwrap();
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(manifest.dependencies[0].id.as_str(), "org.nexus.rust");
    }

    #[test]
    fn approval_requirements_lists_missing_sensitive() {
        let manifest = PluginManifest::parse_toml(SAMPLE).unwrap();
        let granted = crate::permission::GrantedPermissions::grant(
            &manifest.permissions,
            [
                PluginPermission::FilesystemRead,
                PluginPermission::CubeRead,
                PluginPermission::UiRegister,
            ],
        );
        let needed = manifest.approval_requirements(&granted);
        assert!(needed.contains(&PluginPermission::ProcessExecute));
    }
}
