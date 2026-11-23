# Contributing

Full contributing guidelines live in `CONTRIBUTING.md` at the project root
(the `Nexus/` folder). Key points:

1. Implement parts sequentially; verify each against the spec's issue list.
2. Follow the verification pipeline in `CONTRIBUTING.md` (note: on this machine
   `cargo fmt`/`cargo clippy` are blocked by Application Control; the documented
   workarounds are the sanctioned path).
3. Keep this documentation directory current:
   - architecture decisions → `docs/architecture/`
   - wire behavior → `docs/protocol/`
   - new trade-offs → an ADR in `docs/decisions/`

See also: [Coding standards](./coding-standards.md).