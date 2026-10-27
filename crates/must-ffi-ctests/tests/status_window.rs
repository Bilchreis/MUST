// SPDX-License-Identifier: LGPL-3.0-or-later

//! `bGUI` and `SECoP_S_showStatusWindow` open the status TUI, never twice, never failing
//! the call (Linux, with a recording "terminal" in place of a window).

#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use must_ffi_ctests::server as c;
use must_monitor::registry;

/// Run in a child process: the C test, then time for the launch thread.
#[test]
#[ignore = "run in a child process"]
fn status_window_child() {
    // SAFETY: a C test without arguments
    let line = unsafe { c::ctest_status_window() };
    assert_eq!(
        line, 0,
        "C test status_window failed at c/server.c line {line}"
    );
    std::thread::sleep(Duration::from_millis(800));
    println!("pid {}", std::process::id());
}

struct Setup {
    base: PathBuf,
    registry: PathBuf,
    tui: PathBuf,
    terminal: PathBuf,
    record: PathBuf,
}

fn setup(name: &str) -> Setup {
    let base = std::env::temp_dir().join(format!("must-ctests-gui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let registry = base.join("registry");
    registry::prepare_dir(&registry).unwrap();
    let tui = base.join("must-tui");
    std::fs::write(&tui, "").unwrap();
    let record = base.join("terminal-args");
    let terminal = base.join("terminal");
    std::fs::write(
        &terminal,
        format!("#!/bin/sh\necho \"$@\" >> '{}'\n", record.display()),
    )
    .unwrap();
    std::fs::set_permissions(&terminal, std::fs::Permissions::from_mode(0o755)).unwrap();
    Setup {
        base,
        registry,
        tui,
        terminal,
        record,
    }
}

/// Runs the C test in a child with the TUI at `tui`; returns the child's pid.
fn run_child(s: &Setup, tui: &Path) -> u32 {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "status_window_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(registry::DIR_VARIABLE, &s.registry)
        .env("MUST_TUI", tui)
        .env("MUST_TUI_TERMINAL", &s.terminal)
        .env("DISPLAY", ":fake")
        .env("PATH", s.base.join("empty"))
        .env("MUST_LOG_FILES", "0")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{out}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out.lines()
        .find_map(|l| {
            l.split_once("pid ")
                .and_then(|(_, p)| p.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("no pid in {out}"))
}

#[test]
fn first_init_opens_the_tui_focused_on_the_host() {
    let s = setup("first");
    let pid = run_child(&s, &s.tui);
    let record = std::fs::read_to_string(&s.record).unwrap();
    let expected = format!("{} --focus-pid {pid}", s.tui.display());
    assert!(record.lines().count() >= 1, "{record}");
    assert!(record.lines().all(|l| l == expected), "{record}");
    let _ = std::fs::remove_dir_all(&s.base);
}

#[test]
fn already_running_hide_and_missing_executable() {
    let s = setup("running");
    // a TUI runs: nothing is started, and hiding leaves it open
    let tui_lock = registry::try_lock_tui(&s.registry).unwrap().unwrap();
    run_child(&s, &s.tui);
    assert!(!s.record.exists(), "no second TUI");
    assert!(registry::tui_running(&s.registry).unwrap());
    drop(tui_lock);
    // no executable anywhere: the calls still succeed (checked by the C test)
    run_child(&s, &s.base.join("no-such-tui"));
    assert!(!s.record.exists());
    let _ = std::fs::remove_dir_all(&s.base);
}
