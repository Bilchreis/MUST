// SPDX-License-Identifier: LGPL-3.0-or-later

//! `must monitor` runs `must-tui` with the given arguments.

use std::process::Command;

#[test]
fn monitor_runs_the_tui() {
    let out = Command::new(env!("CARGO_BIN_EXE_must"))
        .args(["monitor", "--version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("must-tui "), "{stdout}");
}

#[test]
fn unknown_command_is_a_usage_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_must"))
        .arg("frobnicate")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("usage: must"), "{stderr}");
}

#[test]
fn update_rejects_bad_arguments() {
    for args in [
        &["update", "--force"][..],
        &["update", "--tag"],
        &["update", "--tag", "main", "x"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_must"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "nothing was fetched or installed");
    }
}
