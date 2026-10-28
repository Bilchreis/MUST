// SPDX-License-Identifier: LGPL-3.0-or-later

//! Stands in for `must-tui` in `tests/cross_process.rs`: writes where it runs from and its
//! arguments to the file named by `FAKE_TUI_REPORT`, then exits.

fn main() {
    let Some(report) = std::env::var_os("FAKE_TUI_REPORT") else {
        return;
    };
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = std::fs::write(report, format!("{exe}\n{}\n", args.join(" ")));
}
