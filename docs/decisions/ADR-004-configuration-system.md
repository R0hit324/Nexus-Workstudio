# ADR-004: Configuration System

- **Status:** Accepted
- **Decided:** Part 1
- **Applies to:** all parts

## Context

Operators and developers configure the server and client differently (ports,
data directories, log levels, usernames, server addresses). Configuration must
be discoverable and overridable per environment.

## Decision

- **TOML configuration files**, parsed into typed structs with serde defaults:
  `ServerConfig`, `ClientConfig`, `LoggingConfig`, `NexusConfig`.
- **Precedence:** explicit file path (CLI) → `NEXUS_CONFIG` env var → default
  candidate locations (`dirs::config_dir()/nexus/config.toml`, then
  `./nexus.config.toml`) → built-in defaults.
- **Environment overrides:** `NEXUS_BIND`, `NEXUS_SERVER_PORT`, `NEXUS_DATA_DIR`,
  `NEXUS_LOG_LEVEL`, `NEXUS_SERVER_ADDRESS`, `NEXUS_USERNAME` are applied after
  file loading.
- **CLI flags** (via `clap`) win in the binaries.

## Consequences

- All settings live in a few well-documented structs; partial TOML fills the
  rest from defaults.
- Precedence is deterministic and scriptable (CI, systemd, container envs).
- New settings are added once in the struct with a serde default, then exposed:
  file key, env var, flag.