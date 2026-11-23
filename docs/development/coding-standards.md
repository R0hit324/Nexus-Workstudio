# Coding Standards

## Language and layout

- Rust, current stable. Cargo workspace under `crates/` and `tests/`.
- Every crate owns one concern; dependency direction points toward `nexus-core`
  (see `docs/decisions/ADR-002-service-decomposition.md`).
- Edition 2021, rust-version and version inherited from the workspace root.

## Boundaries

- **No HTTP / REST.** The custom wire protocol in `nexus-protocol` is the only
  transport. Clients and servers agree on framing and capabilities first.
- **Trait boundaries over concrete types.** Domain code talks to the
  `Transport` and `CubeManager` abstractions, not to TCP or store concrete
  types, so those seams stay replaceable.
- **`nexus-core` stays dependency-light.** Networking (tokio), serialization,
  and async plumbing belong to higher crates; core defines the vocabulary.

## Errors

- Use `nexus_core::Error` and its `ErrorCode` ladder everywhere.
- User-facing message + optional `internal` detail (kept server-side).
- Never forward internal detail verbatim to clients; map to wire-level codes
  at the boundary (`nexus-server`).

## Concurrency and async

- `tokio` runtime is created at process entry points (binaries), never in
  library code.
- Per-connection state is independent; shared domain services are behind
  `Arc<dyn Trait>` or explicit locks.
- Prefer cancellable work: bounded timeouts around network receive,
  cooperation with task cancellation.

## Testing

- Unit tests beside their modules; integration tests in `tests/*` crates.
- Tests must be deterministic in wall-clock terms (loose bounds in
  `tests/performance` are intentionally generous).
- Wire behavior is exercised end to end through the public API.

## Formatting and lints

- `rustfmt` (edition 2021) and clippy are mandatory at the 4 suggested levels.
- On this machine, run them per `CONTRIBUTING.md` (the cargo subcommand
  wrappers are blocked by Application Control).