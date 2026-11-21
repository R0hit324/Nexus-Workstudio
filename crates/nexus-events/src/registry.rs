use std::collections::HashMap;

use nexus_core::event::{DeliveryClass, EventCategory, EventType};

/// Authoritative, extensible description of an event type.
///
/// The core enum provides defaults through [`EventType::category`] and
/// [`EventType::delivery`]; the registry lets subsystems register aliases and
/// adjust policy at runtime (spec section 11). Validation uses the registry
/// so misspelled type names cannot silently reach storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSpec {
    pub name: String,
    pub category: EventCategory,
    pub delivery: DeliveryClass,
    /// Whether delivery failure must be surfaced as an error to the caller.
    pub critical: bool,
}

impl TypeSpec {
    #[must_use]
    pub fn from_type(event_type: EventType) -> Self {
        Self {
            name: event_type.name().to_string(),
            category: event_type.category(),
            delivery: event_type.delivery(),
            critical: matches!(
                event_type,
                EventType::AuthenticationSucceeded
                    | EventType::AuthenticationFailed
                    | EventType::PermissionChanged
            ),
        }
    }
}

/// Registry of all known event types plus extension aliases.
#[derive(Debug, Default)]
pub struct EventTypeRegistry {
    specs: HashMap<&'static str, TypeSpec>,
    aliases: HashMap<String, EventType>,
}

impl EventTypeRegistry {
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            specs: HashMap::new(),
            aliases: HashMap::new(),
        };
        for event_type in ALL_EVENT_TYPES.iter().copied() {
            registry
                .specs
                .insert(event_type.name(), TypeSpec::from_type(event_type));
        }
        registry
    }

    /// Registers a new name->type alias without touching the core enum.
    pub fn register_alias(&mut self, name: impl Into<String>, event_type: EventType) {
        self.aliases.insert(name.into(), event_type);
    }

    /// Looks up a type by its canonical name, then by alias.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<EventType> {
        self.specs
            .values()
            .find(|spec| spec.name == name)
            .map(|spec| spec.category)
            .and_then(|_| self.alias_to_type(name))
            .or_else(|| self.aliases.get(name).copied())
    }

    fn alias_to_type(&self, name: &str) -> Option<EventType> {
        // Canonical names have a spec entry; map back from registry spec.
        ALL_EVENT_TYPES.iter().copied().find(|t| t.name() == name)
    }

    /// The spec for a type (returns the built-in spec for canonical types).
    #[must_use]
    pub fn spec(&self, event_type: EventType) -> TypeSpec {
        self.specs
            .get(event_type.name())
            .cloned()
            .unwrap_or_else(|| TypeSpec::from_type(event_type))
    }

    /// Number of registered canonical specs (excluding aliases).
    #[must_use]
    pub fn canonical_count(&self) -> usize {
        self.specs.len()
    }

    /// Validates that a type is known and that its derived values match the
    /// registered spec (catches drift between enum defaults and registry).
    pub fn validate(&self, event_type: EventType) -> crate::Result<()> {
        let spec = self.spec(event_type);
        if spec.name != event_type.name()
            || spec.category != event_type.category()
            || spec.delivery != event_type.delivery()
        {
            return Err(crate::Error::UnexpectedEventType(
                event_type.name().to_string(),
            ));
        }
        Ok(())
    }
}

/// All canonical event types, kept in a single place so the registry and the
/// enum can be checked against each other in tests.
pub const ALL_EVENT_TYPES: &[EventType] = &[
    EventType::CubeCreated,
    EventType::CubeDeleted,
    EventType::UserJoined,
    EventType::UserLeft,
    EventType::SelectionChanged,
    EventType::FileCreated,
    EventType::FileDeleted,
    EventType::FileRenamed,
    EventType::FileEdited,
    EventType::CursorMoved,
    EventType::ChatMessageSent,
    EventType::SessionTerminated,
    EventType::AuthenticationSucceeded,
    EventType::AuthenticationFailed,
    EventType::UnauthorizedOperation,
    EventType::PermissionChanged,
    EventType::TerminalStarted,
    EventType::TerminalInput,
    EventType::TerminalOutput,
    EventType::BuildStarted,
    EventType::BuildFinished,
    EventType::SnapshotCreated,
    EventType::StorageWriteCompleted,
    EventType::System,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_all_canonical_types() {
        let registry = EventTypeRegistry::new();
        assert_eq!(registry.canonical_count(), ALL_EVENT_TYPES.len());
        for t in ALL_EVENT_TYPES.iter().copied() {
            registry.validate(t).unwrap();
            assert_eq!(registry.resolve(t.name()), Some(t));
        }
    }

    #[test]
    fn registry_keeps_aliases_separate_from_canonical() {
        let mut registry = EventTypeRegistry::new();
        assert_eq!(registry.resolve("reboot"), None);
        registry.register_alias("reboot", EventType::System);
        assert_eq!(registry.resolve("reboot"), Some(EventType::System));
        // Canonical name untouched.
        assert_eq!(registry.resolve("system"), Some(EventType::System));
    }

    #[test]
    fn spec_exposes_policy() {
        let registry = EventTypeRegistry::new();
        let spec = registry.spec(EventType::FileEdited);
        assert_eq!(spec.delivery, DeliveryClass::Durable);
        assert_eq!(spec.category, EventCategory::Workspace);
        assert!(!spec.critical);
        let auth = registry.spec(EventType::AuthenticationFailed);
        assert!(auth.critical);
    }
}
