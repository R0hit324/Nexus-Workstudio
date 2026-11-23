# Quick Start, First Run, and Configuration Validation

Part 15 closes its documentation track for the deployment experience:

## Quick start (Part 15 section 47)

```text
Build server binary
Package server
Install server
Configure server
Run server
Verify health
```

## First-run experience (Part 15 section 48)

The first run of a deployment must:

1. Start the server.
2. Complete lifecycle (starting -> initializing -> listening -> running).
3. Pass the health check (liveness + readiness).
4. Accept a client handshake.
5. Report clear, actionable errors on failure.

## Configuration validation (Part 15 section 49)

Before serving traffic, the server validates its effective configuration:

- Required fields present.
- Ports in valid range.
- Data/log directories writable.
- Runtime backend (if set) resolvable.

Invalid configuration fails fast with a clear error rather than at first use.
