# Contributing to Nexus

Nexus is specified part by part. Please read the spec files named
`# Nexus — Part N ...` in the parent directory before starting work on a part.

## Development flow

1. Each part is implemented on its own branch (`part-N-<short-name>`).
2. A part is not "done" until its issue list is checked and the verification
   pipeline below is green.
3. Write tests alongside code. Integration tests go in `tests/*` crates;
   unit tests live beside their modules.
4. Keep the docs up to date: architecture decisions change `docs/architecture`,
   wire behavior changes `docs/protocol`, and new trade-offs deserve an ADR in
   `docs/decisions`.

## Verification pipeline

Run from the workspace root:

```powershell
# 1. Formatting (rustfmt)
# On this machine `cargo fmt` is blocked by Application Control, so rustfmt
# runs directly from the active toolchain:
$toolchain = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc"
rustup toolchain list | Out-Host
# Format check all Rust files:
& "$toolchain\bin\rustfmt.exe" --edition 2021 --check $(Get-ChildItem -Recurse -Filter *.rs | ForEach-Object FullName)

# 2. Static analysis (clippy)
# `cargo clippy` is likewise blocked on this machine. clippy-driver is wired
# in as the workspace wrapper so `cargo check` runs clippy lints:
$env:RUSTC_WORKSPACE_WRAPPER = "$toolchain\bin\clippy-driver.exe"
cargo check --workspace --all-targets

# 3. Tests
cargo test --workspace

# 4. Everything, in order
powershell -ExecutionPolicy Bypass -File scripts\verify.ps1
```

> Environment note: on this developer machine, the Application Control policy
> blocks `cargo-clippy.exe` and `cargo-fmt.exe` (os error 4551). The commands
> above are the sanctioned workarounds and must not be bypassed otherwise.
>
> **Execution blocker (verified 2026-09-11):** the same policy (os error 4551)
> also blocks running *any freshly compiled, unsigned binary* — including every
> `build-script-build.exe` (serde_json, quote, proc-macro2, libc, …) and every
> cargo test/run artifact under `Nexus\target\`. This currently prevents
> `cargo check`, `cargo test`, and `cargo run` outright. Code for all parts is
> written with tests; running the pipeline requires either an App Control
> exception for the workspace `target\` directory (or the placeholder-signing of
> build artifacts) granted by IT, or building on a policy-compliant machine.
> This is a machine security constraint, not a repository defect.

## Coding standards

- No HTTP. No REST. The relational model is by design; reads are belief-tree
  and annotative (`docs/architecture/overview.md`).
- Domain logic depends on `nexus-core` types and the `Transport` abstraction,
  never on TCP details.
- Errors come from the `nexus_core::error` ladder; internal detail stays
  server-side (`.with_internal`) and is never sent to clients verbatim.
- Request/response and event messages belong in `nexus-protocol`;
  `postcard` + length-prefixed frames are the only wire format.
- No secrets in code or config files. Config and env vars are merged in
  `nexus-core::config`.

## Review checklist

- [ ] `rustfmt --check` clean
- [ ] clippy clean via `RUSTC_WORKSPACE_WRAPPER` check
- [ ] `cargo test --workspace` green
- [ ] Spec issue list for the part is complete
- [ ] ADR added when a trade-off was decided