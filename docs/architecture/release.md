# Release and Quality Checklist

Part 14 of the Nexus roadmap requires release testing before every release.
This document defines the gates, the release-candidate procedure, and the
compatibility rules that releases must honor.

## Pre-release gates

Run every gate below before tagging a release:

| # | Gate | Command / evidence |
|---|------|--------------------|
| 1 | Formatting | `cargo fmt --all -- --check` |
| 2 | Lint | `cargo clippy --workspace --all-targets` |
| 3 | Unit and component tests | `cargo test --workspace --lib` |
| 4 | Integration and protocol tests | `cargo test --workspace --tests` |
| 5 | Documentation tests | `cargo test --workspace --doc` |
| 6 | End-to-end scenarios | `tests/integration` scenarios pass |
| 7 | Security tests | authn/authz boundary suite passes |
| 8 | Recovery tests | restart, reconnect, and replay suites pass |
| 9 | Performance regression | `cargo test -p nexus-tests-performance --release` |
| 10 | Build | `cargo build --workspace` |
| 11 | Packaging | release binaries build and are archived |
| 12 | Upgrade path | previous release data loads in the new build |

Gates 7-8, 11, and 12 expand as their Part 14 harnesses land.

## Release candidate procedure

```text
Build release binaries
  -> Run full test suite
  -> Run compatibility tests
  -> Run stress tests
  -> Run recovery tests
  -> Verify package integrity
  -> Verify upgrade path
  -> Sign off
```

Only a candidate that clears every step may be released.

## Versioning and compatibility

- **Semver** for crates and binaries; breaking protocol changes bump the
  major version.
- **Protocol**: client/server negotiate capabilities during the handshake;
  incompatible peers must fail with a clear error, never misinterpret data.
- **Events**: persisted event schemas are versioned; unknown schema versions
  are rejected explicitly.
- **Snapshots**: snapshot format versions are recorded in metadata; old
  snapshots remain loadable or are migrated.
- **Plugins**: plugin API versions are checked against the manifest before
  loading.
- **Configuration**: new fields have defaults; removed fields are reported,
  not silently ignored.

Never silently interpret incompatible data.

## Upgrade testing

For protocol, storage, or snapshot format changes:

```text
Old version -> Upgrade -> New version
```

Verify configuration migration, snapshot migration, event compatibility,
plugin compatibility, and client/server compatibility.

## Failure artifacts

When a gate fails, preserve the artifacts needed to reproduce it:

- logs
- network traces
- event history
- snapshot
- workspace state
- crash information
- build output
- terminal output
- AI tool history (where safe)

## Definition of Done (release)

A release is done when every gate above passes, the candidate procedure is
recorded, and any newly found defect has a regression test in the permanent
regression suite.
