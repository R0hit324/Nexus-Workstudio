# ADR-005: Error Handling and Logging

- **Status:** Accepted
- **Decided:** Part 1
- **Applies to:** all parts

## Context

Distributed, multi-user tooling produces many distinct failure modes. Errors
must be categorized for policy decisions (retry/no-retry, authz vs resource),
while never leaking internal detail to clients. Logs must be machine-readable
in production and human-friendly in development.

## Decision

- **Unified `Error` type** with a stable `ErrorCode` ladder (network,
  authentication, authorization, protocol, not-found, invalid-request,
  already-exists, workspace, storage, runtime, build, terminal, snapshot,
  plugin, ai, internal).
- Every error has a **safe user-facing message** plus an optional `internal`
  detail that stays server-side. Wire-level errors are mapped at the
  server boundary (`nexus-server::message_result`), never forwarded verbatim.
- **Retryable** is derived from the code (network/storage) so call sites can
  decide policy uniformly.
- **Logging** uses `tracing` + `tracing-subscriber` with an env filter and a
  text/JSON format toggle; `try_init` keeps tests safe when repeated.
- Handshake/version mismatches and idle timeouts terminate sessions cleanly.

## Consequences

- Client messages show curated text; operators debug via server-side `internal`
  detail.
- `is_retryable()` gives clients a consistent, code-based retry policy.
- Structured logs pair cleanly with the event stream for post-mortem analysis.