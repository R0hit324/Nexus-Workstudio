# CI pipeline

The Nexus CI definition is [`ci.yml`](./ci.yml) in this directory.

GitHub only executes workflows stored under `.github/workflows/`. To
activate CI, copy `ci.yml` to `.github/workflows/ci.yml` and push with a
credential that has the `workflow` scope (classic PAT) or the
**Workflows: Read and write** repository permission (fine-grained PAT).

All project files live in the `Nexus/` folder at the repository root, so the
workflow sets `defaults.run.working-directory: Nexus`. Keep that block when
copying the file, or every cargo step will run from the wrong directory.

The pipeline stages follow the Part 14 specification:

1. Formatting (rustfmt)
2. Lint (clippy)
3. Unit and component tests
4. Integration and protocol tests
5. Documentation tests
6. Build
7. Performance regression suite (separate release-mode job)

Heavy security, chaos, stress, and soak jobs are added as their harnesses
land, so they do not gate every pull request.
