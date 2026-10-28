// SPDX-License-Identifier: LGPL-3.0-or-later

//! The library opens `must-tui` in a terminal emulator: xterm under Xvfb (Linux, when
//! both are installed; CI installs them).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use must_monitor::registry;

/// Run in a child process by `xterm_under_xvfb`: launches the TUI as the library does
/// and prints the outcome.
#[test]
#[ignore = "run in a child process"]
fn launcher() {
    println!("outcome {:?}", must_server::launcher::launch());
}

fn installed(program: &str) -> bool {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|d| d.join(program).is_file())
}

fn wait_until(what: &str, within: Duration, done: impl Fn() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "{what} not within {within:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Kills the process on drop.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_xvfb() -> (Kill, String) {
    let display = 90 + std::process::id() % 100;
    let xvfb = Command::new("Xvfb")
        .arg(format!(":{display}"))
        .args(["-screen", "0", "1024x768x24", "-nolisten", "tcp"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = PathBuf::from(format!("/tmp/.X11-unix/X{display}"));
    wait_until("Xvfb", Duration::from_secs(10), || socket.exists());
    (Kill(xvfb), format!(":{display}"))
}

fn launch(dir: &Path, display: &str) -> String {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "launcher",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(registry::DIR_VARIABLE, dir)
        .env("MUST_TUI", env!("CARGO_BIN_EXE_must-tui"))
        .env("DISPLAY", display)
        .env("TERMINAL", "xterm")
        .env_remove("MUST_TUI_TERMINAL")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(output.status.success());
    let out = String::from_utf8_lossy(&output.stdout);
    out.lines()
        .find_map(|l| l.split_once("outcome ").map(|(_, o)| o.to_owned()))
        .unwrap_or_else(|| panic!("no outcome in {out}"))
}

#[test]
fn xterm_under_xvfb() {
    if !installed("Xvfb") || !installed("xterm") {
        // CI installs them: there, a missing one is an error
        assert!(
            std::env::var_os("CI").is_none(),
            "Xvfb and xterm are needed in CI"
        );
        println!("Xvfb or xterm not installed: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("must-tui-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    registry::prepare_dir(&dir).unwrap();
    let (_xvfb, display) = start_xvfb();

    let outcome = launch(&dir, &display);
    assert!(outcome.starts_with("Started"), "{outcome}");
    let terminal_pid: u32 = outcome
        .split("pid: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|pid| pid.trim().parse().ok())
        .unwrap();
    // the TUI runs in the terminal: it holds the per-user lock
    wait_until("the TUI", Duration::from_secs(15), || {
        registry::tui_running(&dir).unwrap()
    });
    // a second launch starts nothing
    assert_eq!(launch(&dir, &display), "AlreadyRunning");

    // closing the terminal ends the TUI
    Command::new("kill")
        .arg(terminal_pid.to_string())
        .status()
        .unwrap();
    wait_until("the TUI to end", Duration::from_secs(10), || {
        !registry::tui_running(&dir).unwrap()
    });
    let _ = std::fs::remove_dir_all(&dir);
}
