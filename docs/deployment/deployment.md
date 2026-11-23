# Deployment

Part 15 of the Nexus roadmap introduces deployment, operations, backup,
and release infrastructure. Deployment is treated as a first-class stage of
the product lifecycle: every stage must be documented and testable before the
next one starts.

## Deployment philosophy

Every deployment follows this lifecycle:

```text
Build
  -> Package
  -> Install
  -> Configure
  -> Run
  -> Monitor
  -> Backup
  -> Upgrade
  -> Recover
```

Each stage has an owner, an entry point, and a verification step. A deployment
that cannot pass each stage individually is not releasable.

## Deployment modes

| Mode | Topology | Purpose | Initial release |
|------|----------|---------|-----------------|
| 1. Local development | Developer -> Nexus server -> Local client | Day-to-day development | Yes |
| 2. LAN server | Server -> multiple LAN clients | Primary initial deployment | Yes |
| 3. Dedicated server | Server -> multiple LAN clients (lab/classroom/team) | Hosted single-box deployments | Yes |
| 4. Remote server | Internet/WAN -> server -> remote clients | Future | No (not required initially) |

The initial release ships modes 1-3 and keeps mode 4 explicitly optional.

## Server packaging

The standalone server binary is `nexus-server`.

Requirements:

- The binary must not require the Rust toolchain on the target machine.
- Package: binary, configuration, default directories, service definition,
  documentation.

Default runtime directories:

- Server data (`ServerConfig.data_dir`, default `nexus-data`).
- Event log directory.
- Snapshot store directory.

## Client packaging

The standalone client binary is `nexus` (alias of the TUI client build).
The client should be installable independently from the server where practical.

## Versioning and compatibility

Use semantic versioning (`MAJOR.MINOR.PATCH`).

Compatibility is defined between:

- Client and server.
- Protocol version and capabilities negotiated during handshake.
- Snapshot schema versions.
- Event schema versions.
- Plugin API versions.
- AI provider contracts.
- Runtime images and build adapters.

Breaking changes bump the major version; additive changes bump the minor
version; fixes bump the patch version.

Never silently interpret incompatible data. Incompatible peers must fail with
a clear error during the handshake rather than misinterpreting payloads.

## Configuration

Deployment-sensitive configuration lives in:

- Global configuration.
- Server configuration (TOML, defaults documented).
- Environment variables (`NEXUS_CONFIG`, `NEXUS_DATA_DIR`, `NEXUS_LOG_LEVEL`,
  `NEXUS_BIND`, `NEXUS_PORT`).
- Command-line overrides.

Precedence is: explicit CLI override, environment variable, server config file,
built-in default.
