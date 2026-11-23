# Storage, Snapshot, and Event Migrations

Part 15 treats schema evolution as a first-class deployment concern. Every
persistent format has an explicit version, and upgrades go through the
migration registry instead of ad-hoc conversion code.

## Version constants

| Constant | Lives in | Current value |
|----------|---------|---------------|
| `PROTOCOL_VERSION` | `nexus-core::version` | `1` |
| `WORKSPACE_VERSION` | `nexus-core::version` | `1` |
| `EVENT_SCHEMA_VERSION` | `nexus-core::version` / `nexus-events` | `1` |
| `SNAPSHOT_VERSION` | `nexus-core::version` / `nexus-snapshot` | `1` |
| `PLUGIN_API_VERSION` | `nexus-core::version` | `1` |
| `SYNC_PROTOCOL_VERSION` | `nexus-protocol` | `1` |

The server never assumes that an installed version equals the latest version;
clients and servers negotiate explicitly during the handshake, and persistent
structures record the version they were written with.

## Migration registry

`nexus_core::MigrationRegistry` plans forward-only upgrades:

```rust
let mut registry = MigrationRegistry::new();
registry.register(2, "add event payload hash")?;
let step = registry.next(1); // Some(MigrationStep { version: 2, .. })
registry.next(2); // None once the registry is up to date
```

Rules enforced by the registry:

1. Versions must strictly increase; duplicate or downgrade registrations are
   rejected.
2. `next(from_version)` returns the first step after the installed version and
   walks the forward plan.
3. Migrations never downgrade. Downgrades go through the documented rollback
   procedure instead.

## Per-domain migration tracks

- **Event compatibility** (`nexus-events`, `EVENT_SCHEMA_VERSION`): event
  payloads are versioned; unknown schema versions fail closed rather than being
  silently reinterpreted (Part 14 security boundary coverage).
- **Snapshot migration** (`nexus-snapshot`, `SNAPSHOT_VERSION`): snapshots carry
  their schema version; validation rejects unsupported versions.
- **Storage migration** (`nexus-core` config + storage layer): data directories
  are versioned through configuration; layout changes land as additive steps in
  the registry.

## Upgrade procedure

1. Stop the server (graceful shutdown).
2. Back up (see `docs/deployment/backup.md`).
3. Apply the forward migration plan for the installed version.
4. Start the server and run the health checks (liveness + readiness).
5. Verify with recovery/snapshot loads and the regression suites.

A migration that cannot be completed must leave the previous data untouched;
partial state is detected by checksums and validation before recovery.
