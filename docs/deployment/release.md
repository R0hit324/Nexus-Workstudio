# Release Pipeline and Release Checklist

Part 15 closes the deployment phase's release engineering track:

- **Release pipeline** (Part 15 section 44): reproducible builds, package
  integrity, dependency management, package cache, update mechanism.
- **Release checklist** (Part 15 section 45): final release sign-off covering
  build, package, install, configuration validation, backup, health check, and
  rollback readiness.

## Release pipeline stages

```text
Source
  -> Reproducible build
  -> Package (binary + config + default dirs + service definition)
  -> Integrity check (checksums, signatures)
  -> Publish artifact
  -> Stage release candidate
```

## Release checklist

Before any release candidate is promoted:

1. Build succeeds reproducibly.
2. Package integrity verified.
3. Configuration validates.
4. Backup taken.
5. Health check passes (liveness + readiness).
6. Smoke test (server starts, accepts client, performs handshake).
7. Rollback plan recorded.
8. Release notes drafted.

A release candidate that fails any step is not releasable.
