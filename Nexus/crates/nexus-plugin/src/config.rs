//! Plugin configuration schema and validation.
//!
//! Plugins declare a small typed schema (booleans, integers, strings,
//! enumerations). Configuration the host supplies is namespaced by plugin
//! id; invalid values produce a `configuration.error` instead of crashing
//! the plugin (spec sections 21–22).

use std::collections::BTreeMap;

use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;

/// The kind of one configuration value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigKind {
    Bool,
    Int,
    Str,
    /// One of a fixed set (validated by membership).
    Enum(Vec<String>),
}

/// A field in a plugin's configuration schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigField {
    pub name: String,
    pub kind: ConfigKind,
    pub required: bool,
    /// Human one-liner used by settings UIs.
    pub description: String,
}

/// A small validated configuration schema.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigSchema {
    pub fields: Vec<ConfigField>,
}

/// A raw configuration value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    Str(String),
    /// An unknown shape carried through and reported as an error.
    Other(String),
}

impl ConfigValue {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Str(_) => "string",
            Self::Other(_) => "other",
        }
    }
}

impl ConfigSchema {
    /// Validates a configuration map against the schema.
    ///
    /// # Errors
    /// [`PluginError`] with operation `configuration.error` on any mismatch:
    /// missing required keys, type mismatches, unknown keys, enum members
    /// outside the declared set.
    pub fn validate(
        &self,
        plugin: &PluginId,
        values: &BTreeMap<String, ConfigValue>,
    ) -> PluginResult<BTreeMap<String, ConfigValue>> {
        let mut resolved = BTreeMap::new();
        for field in &self.fields {
            let provided = values.get(&field.name);
            match provided {
                Some(value) => {
                    resolve_value(plugin, field, value)?;
                    resolved.insert(field.name.clone(), value.clone());
                }
                None if field.required => {
                    return Err(ConfigSchema::field_error(
                        plugin,
                        field,
                        String::from("missing required value"),
                    ));
                }
                None => {}
            }
        }
        for key in values.keys() {
            if !self.fields.iter().any(|f| &f.name == key) {
                let mut known: Vec<&str> = self.fields.iter().map(|f| f.name.as_str()).collect();
                known.sort_unstable();
                let expected = if known.is_empty() {
                    String::from("<none>")
                } else {
                    known.join(", ")
                };
                return Err(PluginError::new(
                    Some(plugin.clone()),
                    "configuration.error",
                    format!("unknown configuration key `{key}` (expected: {expected})"),
                ));
            }
        }
        Ok(resolved)
    }

    fn field_error(plugin: &PluginId, field: &ConfigField, detail: String) -> PluginError {
        PluginError::new(
            Some(plugin.clone()),
            "configuration.error",
            format!("`{}`: {}", field.name, detail),
        )
    }
}

fn resolve_value(plugin: &PluginId, field: &ConfigField, value: &ConfigValue) -> PluginResult<()> {
    let expected = match &field.kind {
        ConfigKind::Bool => "bool",
        ConfigKind::Int => "int",
        ConfigKind::Str => "string",
        ConfigKind::Enum(_) => "enum",
    };
    let kind_ok = match (&field.kind, value) {
        (ConfigKind::Bool, ConfigValue::Bool(_)) => true,
        (ConfigKind::Int, ConfigValue::Int(_)) => true,
        (ConfigKind::Str, ConfigValue::Str(_)) => true,
        (ConfigKind::Enum(_), ConfigValue::Str(actual)) => {
            if let ConfigKind::Enum(allowed) = &field.kind {
                if allowed.contains(actual) {
                    true
                } else {
                    return Err(ConfigSchema::field_error(
                        plugin,
                        field,
                        format!("value `{actual}` not in enum {allowed:?}"),
                    ));
                }
            } else {
                false
            }
        }
        (
            ConfigKind::Bool | ConfigKind::Int | ConfigKind::Str | ConfigKind::Enum(_),
            ConfigValue::Other(shape),
        ) => {
            return Err(ConfigSchema::field_error(
                plugin,
                field,
                format!("unsupported configuration type `{shape}`"),
            ));
        }
        _ => false,
    };
    if kind_ok {
        Ok(())
    } else {
        Err(ConfigSchema::field_error(
            plugin,
            field,
            format!("expected kind `{expected}`, found `{}`", value.kind_name()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> PluginId {
        PluginId::new("org.nexus.git").unwrap()
    }

    fn sample_schema() -> ConfigSchema {
        ConfigSchema {
            fields: vec![
                ConfigField {
                    name: String::from("auto_fetch"),
                    kind: ConfigKind::Bool,
                    required: false,
                    description: String::from("periodic fetch"),
                },
                ConfigField {
                    name: String::from("interval"),
                    kind: ConfigKind::Int,
                    required: false,
                    description: String::from("seconds"),
                },
                ConfigField {
                    name: String::from("remote"),
                    kind: ConfigKind::Str,
                    required: true,
                    description: String::from("remote name"),
                },
            ],
        }
    }

    #[test]
    fn accepts_well_typed_values() {
        let mut values = BTreeMap::new();
        values.insert(String::from("auto_fetch"), ConfigValue::Bool(true));
        values.insert(String::from("interval"), ConfigValue::Int(300));
        values.insert(
            String::from("remote"),
            ConfigValue::Str(String::from("origin")),
        );
        let resolved = sample_schema().validate(&plugin(), &values).unwrap();
        assert_eq!(resolved.len(), 3);
    }

    #[test]
    fn rejects_missing_required() {
        let values = BTreeMap::new();
        let err = sample_schema().validate(&plugin(), &values).unwrap_err();
        assert!(err.message.contains("`remote`: missing required value"));
        assert_eq!(err.operation, "configuration.error");
    }

    #[test]
    fn rejects_wrong_types() {
        let mut values = BTreeMap::new();
        values.insert(
            String::from("interval"),
            ConfigValue::Str(String::from("far")),
        );
        let err = sample_schema().validate(&plugin(), &values).unwrap_err();
        assert!(err
            .message
            .contains("`interval`: expected kind `int`, found `string`"));
    }

    #[test]
    fn enum_membership_enforced() {
        let schema = ConfigSchema {
            fields: vec![ConfigField {
                name: String::from("level"),
                kind: ConfigKind::Enum(vec![String::from("info"), String::from("debug")]),
                required: true,
                description: String::from("log level"),
            }],
        };
        let mut bad = BTreeMap::new();
        bad.insert(
            String::from("level"),
            ConfigValue::Str(String::from("trace")),
        );
        assert!(schema.validate(&plugin(), &bad).is_err());
        let mut good = BTreeMap::new();
        good.insert(
            String::from("level"),
            ConfigValue::Str(String::from("debug")),
        );
        assert!(schema.validate(&plugin(), &good).is_ok());
    }

    #[test]
    fn unknown_keys_reported_with_expected_list() {
        let mut values = BTreeMap::new();
        values.insert(
            String::from("remote"),
            ConfigValue::Str(String::from("origin")),
        );
        values.insert(String::from("wat"), ConfigValue::Bool(true));
        let err = sample_schema().validate(&plugin(), &values).unwrap_err();
        assert!(err.message.contains("unknown configuration key `wat`"));
    }
}
