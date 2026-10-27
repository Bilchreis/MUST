// SPDX-License-Identifier: LGPL-3.0-or-later

//! Only one `must-tui` runs per user.

use std::process::Command;

#[test]
fn second_instance_exits_with_a_message() {
    let dir = std::env::temp_dir().join(format!("must-tui-single-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    must_monitor::registry::prepare_dir(&dir).unwrap();
    // the running TUI holds the lock (as must-tui does at start)
    let first = must_monitor::registry::try_lock_tui(&dir).unwrap().unwrap();

    let second = Command::new(env!("CARGO_BIN_EXE_must-tui"))
        .env(must_monitor::registry::DIR_VARIABLE, &dir)
        .output()
        .unwrap();
    assert!(!second.status.success());
    let message = String::from_utf8_lossy(&second.stderr);
    assert!(message.contains("already running"), "{message}");
    assert!(second.stdout.is_empty(), "the terminal is not touched");

    // once the first is gone, a new one may start (it then fails here only for lack of
    // a terminal, after taking the lock)
    drop(first);
    let third = Command::new(env!("CARGO_BIN_EXE_must-tui"))
        .env(must_monitor::registry::DIR_VARIABLE, &dir)
        .output()
        .unwrap();
    let message = String::from_utf8_lossy(&third.stderr);
    assert!(!message.contains("already running"), "{message}");
    let _ = std::fs::remove_dir_all(&dir);
}
