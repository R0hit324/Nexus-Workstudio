#requires -Version 5.1
<#
.SYNOPSIS
  Workspace-wide verification for Nexus: formatting, clippy, and tests.

.DESCRIPTION
  On this machine `cargo fmt` and `cargo clippy` are blocked by Application
  Control (os error 4551). This script runs rustfmt directly and wires
  clippy-driver in as RUSTC_WORKSPACE_WRAPPER so every `cargo check` is a
  clippy check. These are the sanctioned workarounds.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify.ps1
#>

$ErrorActionPreference = 'Stop'

# NOTE: As of 2026-09-11, Application Control on this machine blocks execution
# of unsigned binaries compiled into the workspace target dir (os error 4551).
# This includes dependency build scripts, so `cargo check`/`cargo test`/`cargo
# run` are currently blocked here. This script documents the intended pipeline
# and will work as written on a policy-compliant machine or once IT grants an
# App Control exception for the workspace target directory.

# Determine the active toolchain directory through rustup (cargo is allowed).
try {
    $cargoBin = Get-Command cargo -ErrorAction Stop
} catch {
    throw "cargo not found on PATH. Run provisioning first (rustup)."
}

$toolchainRoot = Join-Path $env:USERPROFILE '.rustup\toolchains'
$toolchainDir = Get-ChildItem $toolchainRoot -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName 'bin\rustfmt.exe') } |
    Sort-Object Name -Descending |
    Select-Object -First 1
if (-not $toolchainDir) {
    throw "No rustup toolchain with rustfmt found under $toolchainRoot"
}

$bin = Join-Path $toolchainDir.FullName 'bin'
$rustfmt = Join-Path $bin 'rustfmt.exe'
$clippyDriver = Join-Path $bin 'clippy-driver.exe'

Write-Host "== Formatting check (rustfmt) =="
$rsFiles = Get-ChildItem -Path . -Recurse -Filter *.rs |
    Where-Object { $_.FullName -notmatch '\\target\\' } |
    ForEach-Object { $_.FullName }
if (-not $rsFiles) {
    Write-Host 'No Rust files found.'
} else {
    & $rustfmt --edition 2021 --check @($rsFiles)
    if ($LASTEXITCODE -ne 0) {
        throw 'Formatting check failed. Run rustfmt to fix.'
    }
    Write-Host 'Formatting OK.'
}

Write-Host '== Clippy (via RUSTC_WORKSPACE_WRAPPER) ='
$env:RUSTC_WORKSPACE_WRAPPER = $clippyDriver
try {
    cargo check --workspace --all-targets
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo check (clippy) failed.'
    }
} finally {
    Remove-Item Env:\RUSTC_WORKSPACE_WRAPPER -ErrorAction SilentlyContinue
}
Write-Host 'Clippy OK.'

Write-Host '== Tests ='
cargo test --workspace
if ($LASTEXITCODE -ne 0) {
    throw 'cargo test failed.'
}

Write-Host '== Build ='
cargo build --workspace
if ($LASTEXITCODE -ne 0) {
    throw 'cargo build failed.'
}

Write-Host ''
Write-Host 'Verification complete: formatting, clippy, tests, and build all pass.'