// SPDX-License-Identifier: LGPL-3.0-or-later

//! Windows: a process started with a console of its own writes to that console, not to
//! ours, and ending it leaves us running.

#![cfg(windows)]

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use must_monitor::platform;

/// Run in the new console by `own_console`: reports whether its output is a console,
/// then waits to be ended.
#[test]
#[ignore = "run in a child process"]
fn console_child() {
    let report = PathBuf::from(std::env::var_os("MUST_TEST_CONSOLE_REPORT").unwrap());
    let terminal = std::io::stdout().is_terminal() && std::io::stdin().is_terminal();
    std::fs::write(&report, if terminal { "console" } else { "no console" }).unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn own_console() {
    let report = std::env::temp_dir().join(format!("must-console-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&report);
    // the child inherits our environment
    // SAFETY: no other thread of this test binary reads the environment meanwhile
    unsafe {
        std::env::set_var("MUST_TEST_CONSOLE_REPORT", &report);
    }
    let exe = std::env::current_exe().unwrap();
    let args: Vec<String> = ["--ignored", "--exact", "console_child", "--test-threads=1"]
        .map(str::to_owned)
        .to_vec();
    let pid = platform::spawn_in_new_console(&exe, &args).unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    while !report.exists() {
        assert!(Instant::now() < deadline, "the child did not report");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(std::fs::read_to_string(&report).unwrap(), "console");
    assert!(platform::process(pid).unwrap().is_some(), "running");

    // ending it (as closing its window does) leaves us running
    let killed = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .unwrap();
    assert!(killed.status.success(), "{killed:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while platform::process(pid).unwrap().is_some() {
        assert!(Instant::now() < deadline, "the child did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&report);
}
