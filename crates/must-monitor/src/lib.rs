// SPDX-License-Identifier: LGPL-3.0-or-later

//! Monitoring of MUST SEC nodes.
//!
//! Every process running MUST nodes opens one local monitoring endpoint (a Unix domain
//! socket on Linux, a named pipe on Windows) and registers it in a per-user directory.
//! This crate holds what both sides share: the [`protocol`], the [`registry`] of
//! endpoints, the local [`transport`], and the [`client`] library that tools such as
//! `must-tui` use to discover and read endpoints. The endpoint itself is part of
//! `must-server`.

/// File name of the status TUI executable, `must-tui`.
pub const TUI_EXECUTABLE: &str = if cfg!(windows) {
    "must-tui.exe"
} else {
    "must-tui"
};

pub mod client;
pub mod platform;
pub mod protocol;
pub mod registry;
pub mod transport;
