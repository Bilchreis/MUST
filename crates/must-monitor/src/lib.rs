// SPDX-License-Identifier: LGPL-3.0-or-later

//! Monitoring of must SEC nodes.
//!
//! Every process running must nodes opens one local monitoring endpoint (a Unix domain
//! socket on Linux, a named pipe on Windows) and registers it in a per-user directory.
//! This crate holds what both sides share: the [`protocol`], the [`registry`] of
//! endpoints, the local [`transport`], and the [`client`] library that tools such as
//! `must-tui` use to discover and read endpoints. The endpoint itself is part of
//! `must-server`.

pub mod client;
pub mod platform;
pub mod protocol;
pub mod registry;
pub mod transport;

/// Held by tests that spawn processes and by tests of file locks. A child process
/// inherits all open files between fork and exec, so a lock file this process just
/// closed stays locked (`flock` belongs to the open file) until the child has started.
#[cfg(test)]
pub(crate) fn spawn_guard() -> std::sync::MutexGuard<'static, ()> {
    static SPAWN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SPAWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
