// SPDX-License-Identifier: LGPL-3.0-or-later

//! The TUI lock is released with the file that holds it.
//!
//! A test binary of its own: on Unix, a child being spawned holds a copy of every open
//! file until it reaches `exec`, and with it the lock. A test spawning children in
//! parallel would make the lock seem held just after its release.

use must_monitor::registry::{try_lock_tui, tui_running};

#[test]
fn tui_lock() {
    let dir = std::env::temp_dir().join(format!("must-tui-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(!tui_running(&dir).unwrap());
    let held = try_lock_tui(&dir).unwrap().expect("free");
    assert!(tui_running(&dir).unwrap());
    assert!(try_lock_tui(&dir).unwrap().is_none());
    drop(held);
    assert!(!tui_running(&dir).unwrap(), "released with the file");
    let _ = std::fs::remove_dir_all(&dir);
}
