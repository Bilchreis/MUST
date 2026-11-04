# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Installs the `must` command and the status TUI `must-tui` from this checkout into
# cargo's bin directory (%USERPROFILE%\.cargo\bin). `must update` later installs the newest release.
# Afterwards: `must monitor`.
$ErrorActionPreference = 'Stop'

Set-Location $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo not found. Install Rust from https://rustup.rs, then run this again."
    exit 1
}

cargo install --locked --path crates/must-tui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$bin = Join-Path $cargoHome 'bin'
if (($env:PATH -split ';') -contains $bin) {
    Write-Output 'Installed. Run `must monitor`; `must update` installs new releases.'
} else {
    Write-Output "Installed into $bin, which is not on your PATH. Add it in the system settings (Environment Variables)."
}
