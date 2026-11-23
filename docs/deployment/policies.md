# Release Notes, Deprecation, Supply Chain, and Privacy Policy

Part 15 sections 44-46, 88-92 plus critical deployment rules.

## Release notes process

Every release produces notes with:

1. Summary of changes by surface (server, client, protocol, storage, snapshot,
   events, plugins, AI).
2. Migration steps required by version delta.
3. Rollback instructions.
4. Verification checklist (build, package, install, config, backup, health,
   smoke test).

A release candidate that fails any step of the release checklist
(`docs/deployment/release.md`) is not releasable.

## API stability and deprecation policy

- Public APIs are semver-covered; breaking changes need a migration path and
  a major version bump.
- Deprecated items remain functional through the current minor cycle and are
  removed only on a major boundary.
- Additive changes bump the minor version; fixes bump the patch version.

## Supply-chain security

- Release artifacts must be integrity-verifiable (checksums, signatures).
- Reproducible builds are investigated where practical; dependency pinning
  reviewed per release.
- CI pipeline (see `docs/ci`) runs format/lint/test/build/perf gates.

## Privacy

- Diagnostics and crash reports must not capture secrets, tokens, or payload
  contents; redaction rules enforced before any bundle leaves the host.
- Log rotation and retention apply to all formats (text/json).

## Security policy alignment

- Secure defaults over convenience (critical deployment rule 1).
- Never deploy without a tested backup and recovery path (rule 2).
- Never expose secrets through logs, diagnostics, or snapshots (rule 4).
- Never assume a LAN is trusted (rules 5, 26).
- Server authorization remains authoritative (rule 6).
- Client-side checks are not sufficient (rule 7).
