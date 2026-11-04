#!/bin/sh
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Installs the `must` command and the status TUI `must-tui` from this checkout into
# cargo's bin directory (~/.cargo/bin). `must update` later installs the newest release.
# Afterwards: `must monitor`.
set -eu

cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
    echo "install.sh: cargo not found. Install Rust from https://rustup.rs, then run this again." >&2
    exit 1
fi

cargo install --locked --path crates/must-tui

bin="${CARGO_HOME:-$HOME/.cargo}/bin"
case ":$PATH:" in
    *":$bin:"*) echo "Installed. Run \`must monitor\`; \`must update\` installs new releases." ;;
    *) echo "Installed into $bin, which is not on your PATH. Add it (e.g. in ~/.profile):"
       echo "    export PATH=\"$bin:\$PATH\"" ;;
esac
