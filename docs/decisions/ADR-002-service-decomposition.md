# ADR-002: Service-Oriented Decomposition in the Workspace

- **Status:** Accepted
- **Decided:** Part 1
- **Applies to:** all parts

## Context

A 15-part spec defines distinct concerns (core, protocol, server, client, TUI,
workspace, events, snapshots, terminals, builds, runtimes, plugins, AI). One
monolithic binary or one giant crate would defeat testability and phase-by-phase
delivery.

## Decision

- One crate per concern under `crates/`, one test crate per test concern under
  `tests/`.
- `nexus-core` is the dependency root: config, logging, errors, ids, events,
  session/workspace/cube vocabulary. It carries no tokio or socket code.
- Higher crates depend on abstractions from `nexus-core` (`CubeManager`,
  `Workspace`) and `nexus-protocol` (`Transport`) rather than on each other's
  internals.
- Unstarted concerns ship as minimal placeholder crates so `cargo test
  --workspace` is green from Part 1 onward.

## Consequences

- Parallel phase work is possible; a phase typically fills in one crate and
  wires tests in the matching `tests/*` crate.
- The crate boundary enforces the "no HTTP, no REST, transport-agnostic" rule.
- Placeholder crates must be updated to real implementations (and, where
  appropriate, retired or extended in later parts).