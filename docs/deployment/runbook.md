# Operational Runbook, Incident Response, and Rollback

Part 15 section 93-95: operational procedures for a deployed Nexus instance.

## Normal operations checklist

1. Server up: lifecycle Running, liveness true, readiness true.
2. Health: `nexus_server::diagnostics` report shows expected counters.
3. Logs: structured per configured format; rotation handled externally unless
   a file appender is configured.
4. Backups: taken per retention policy; last verified backup recorded.

## Startup / shutdown

- Startup: config validation (`NexusConfig::validate`) → lifecycle
  Initializing → Listening → Running → ready at `local_addr`.
- Shutdown: `Shutdown::trigger` (SIGINT/SIGTERM) → lifecycle Stopping → Stopped;
  connections drain before accept loop exit.

## Backup and restore

- Backup scope: storage, event log, snapshots, runtime state.
- Procedure: stop server (or use consistent snapshot) → archive → verify
  checksum → restore to target → verify restored stores load.

## Upgrade procedure

1. Backup.
2. Stop server gracefully.
3. Apply forward migrations (registry plan).
4. Start, health check, smoke test (handshake + client create/list).

## Incident response

1. Detect: metrics/logs/health probes alert.
2. Contain: stop accepting new sessions if needed; rollback path below.
3. Diagnose: gather diagnostic report + crash reports (panic hook) + logs +
   diagnostic bundle.
4. Resolve: restore from verified backup or complete forward migration.
5. Postmortem: record timeline; confirmed defects enter regression suite via
   stable IDs.

## Rollback

1. Stop server.
2. Restore previous artifact + previous data (backup).
3. Validate: start, health check, recovery load, smoke test.
4. Only keep new data if migration fully verified; never perform irreversible
   migrations without rollback planning (critical deployment rule 3).

## Disaster recovery

- Restore drill required before any release; backups verified by restoration.
- Recovery uses snapshot + event log replay (`RecoveryManager`).

## Severity ladder

- Sev1: server unreachable / data loss — page, rollback, notify.
- Sev2: degraded (health failing) — investigate with diagnostics.
- Sev3: functional bug — ticket, fix, regression suite.
