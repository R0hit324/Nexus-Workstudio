# Compatibility Matrix and Release Policies

Part 15 sections 5-8, 44-46, 88-92 and critical deployment rules require an
explicit compatibility contract and policy surface.

## Compatibility matrix

Compatibility is defined across every versioned surface; incompatible peers
must fail during the handshake rather than misinterpreting data.

| Surface | Versioned by | Current | Breaking change policy |
|---------|--------------|---------|------------------------|
| Client ↔ Server wire protocol | `PROTOCOL_VERSION` (`nexus-core::version`, `nexus-protocol`) | `1` | Major bump + migration path |
| Client ↔ Server capabilities | handshake negotiation | `1` | Additive-only per minor |
| Workspace model | `WORKSPACE_VERSION` | `1` | Major bump + migration |
| Event schema | `EVENT_SCHEMA_VERSION` | `1` | Major bump + migration |
| Snapshot format | `SNAPSHOT_VERSION` (`nexus-snapshot`) | `1` | Major bump + migration |
| Plugin API | `PLUGIN_API_VERSION` | `1` | Major bump + migration |
| AI provider contract | provider registry version | `1` | Additive-only per minor |
| Sync protocol | `SYNC_PROTOCOL_VERSION` | `1` | Major bump + migration |
| Build adapter / runtime images | release tag | per release | Additive-only per minor |

Clients and servers built against different major versions of any surface
must not interoperate; they must negotiate or refuse to connect.

## Release policies

- **Release notes**: every release ships notes covering added/removed/changed
  surfaces, migration steps, and rollback instructions.
- **API stability**: public Rust APIs are covered by semver; breaking changes
  require a migration path and a major version bump.
- **Deprecation**: deprecated surfaces carry a removal milestone and a
  replacement; removals happen only at major boundaries.
- **Supply-chain security**: release artifacts must be integrity-verifiable
  (checksums/signatures); reproducible builds are investigated per release.
- **Privacy**: diagnostics and crash reports must never capture secrets; logs
  and reports must redact tokens and payload contents.

## Policy enforcement points

- Handshake version mismatch → `ProtocolMismatch` (fail closed).
- Snapshot validation → unsupported schema version → rejection.
- Event schema check → unknown schema version → reject.
- Migration registry → forward-only steps; rollback is procedural.
