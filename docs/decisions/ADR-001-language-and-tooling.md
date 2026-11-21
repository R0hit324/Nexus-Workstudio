# ADR-001: Language, Tooling, and Async Runtime

- **Status:** Accepted
- **Decided:** Part 1
- **Applies to:** all parts

## Context

Nexus needs a systems language with first-class async networking, strong
typing, a good TUI ecosystem, and low-friction deployment for a small team's
self-hosted LAN server.

## Decision

- **Rust** (stable, 2021 edition) as the sole implementation language.
- **Cargo workspace** with one crate per concern.
- **Tokio** for the async runtime and networking.
- `tracing` (with `tracing-subscriber`) for structured logging.
- Semver-clean dependency policy: prefer few, well-maintained dependencies;
  keep `nexus-core` deliberately dependency-light.

## Consequences

- Shared `[workspace.dependencies]` keeps versions consistent across crates.
- Cargo's build/test/clippy/fmt round trip is the verification backbone.
- The async model forces edge cases (cancellation, timeouts) into the open
  early, where they belong.
- On the authoring machine, Application Control blocks `cargo fmt`/`cargo
  clippy` subcommands; documented workarounds are used instead (see
  `CONTRIBUTING.md`).