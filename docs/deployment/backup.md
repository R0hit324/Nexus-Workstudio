# Backup and Recovery

Part 15 ties deployment operations together: backup strategy, backup
commands, and backup verification feed directly into the disaster recovery
and rollback stages of the deployment lifecycle.

## Backup strategy

The server must back up, at minimum:

- Server storage (workspace data, file contents).
- Event log (append-only event history required for replay/recovery).
- Snapshot store (durable snapshots).
- Runtime state (plugin data, AI tool history where safe, caches).

Backup rules:

- Backups are taken while the server is running where supported; otherwise
  from a stopped instance.
- Retention and rotation policy is configurable; the default keeps a bounded
  number of snapshots/backups.
- Backups live outside the live storage directory (separate volume/prefix)
  so a bad deploy cannot destroy the last good copy.

## Backup commands

The deployment CLI exposes:

```text
nexus-server backup
nexus-server restore
```

- `backup` writes a consistent point-in-time archive of the above state.
- `restore` materializes a backup into a target data directory.

## Backup verification

Every backup must be verifiable before restore:

- Checksum of the archive.
- Restore drill into a temporary environment.
- Verify the restored event store and snapshot stores load cleanly.

Only verified backups may be used for recovery.

## Disaster recovery and rollback

See `docs/deployment/deployment.md` for the full lifecycle ordering
(Backup before Upgrade; Validate after Restore; Keep the original until the
migration succeeds).
