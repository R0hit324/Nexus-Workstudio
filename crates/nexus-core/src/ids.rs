use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generates a strongly typed newtype id over a `Uuid`.
///
/// Every generated type is `Copy`, comparable, hashable, and serializable so
/// that ids can be stored, compared, and carried over the network protocol.
macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Returns the underlying [`Uuid`].
            #[must_use]
            pub fn as_uuid(self) -> Uuid {
                self.0
            }

            /// Builds an id from a 64-bit value. Used for sequentially assigned
            /// ids (e.g. client `RequestId`s). The high 64 bits are zero.
            #[must_use]
            pub fn from_u64(value: u64) -> Self {
                Self(Uuid::from_u64_pair(0, value))
            }

            /// Returns the 64-bit value when this id was built by
            /// [`Self::from_u64`], otherwise `None`.
            #[must_use]
            pub fn to_u64(self) -> Option<u64> {
                let (hi, lo) = self.0.as_u64_pair();
                if hi == 0 { Some(lo) } else { None }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

id_type! {
    /// Uniquely identifies a user account.
    UserId
}

id_type! {
    /// Uniquely identifies an authenticated connection/session.
    SessionId
}

id_type! {
    /// Uniquely identifies a live network connection, independent of the
    /// logical session bound to it. Kept separate from [`SessionId`] so a
    /// network disconnect does not destroy a recoverable session.
    ConnectionId
}

id_type! {
    /// Uniquely identifies the central unit of Nexus: an isolated development
    /// environment / project.
    CubeId
}

id_type! {
    /// Uniquely identifies a workspace (the file tree of a Cube).
    WorkspaceId
}

id_type! {
    /// Uniquely identifies a file within a workspace.
    FileId
}

id_type! {
    /// Uniquely identifies an event.
    EventId
}

id_type! {
    /// Uniquely identifies a workspace snapshot.
    SnapshotId
}

id_type! {
    /// Uniquely identifies a Cube runtime instance.
    RuntimeId
}

id_type! {
    /// Uniquely identifies a terminal session.
    TerminalId
}

id_type! {
    /// Uniquely identifies a project subsystem indirectly related to a Cube.
    ProjectId
}

id_type! {
    /// Uniquely identifies a build.
    BuildId
}

id_type! {
    /// Uniquely identifies a build artifact.
    ArtifactId
}

id_type! {
    /// Uniquely identifies a request so responses can be correlated.
    RequestId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_newtype_roundtrips_through_string() {
        let id = UserId::new();
        let text = id.to_string();
        let parsed = text.parse::<UserId>().expect("parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_are_unique_in_practice() {
        let a = CubeId::new();
        let b = CubeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_serialize_through_json() {
        let id = EventId::new();
        let encoded = serde_json::to_string(&id).expect("encode");
        let decoded: EventId = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(id, decoded);
    }

    #[test]
    fn ids_convert_to_and_from_uuid() {
        let uuid = Uuid::new_v4();
        let id: SessionId = uuid.into();
        assert_eq!(Uuid::from(id), uuid);
    }
}
