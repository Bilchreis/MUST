// SPDX-License-Identifier: LGPL-3.0-or-later

//! Monitoring of MUST SEC nodes.
//!
//! Every process running MUST nodes opens one local monitoring endpoint (a Unix domain
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
