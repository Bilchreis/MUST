// SPDX-License-Identifier: LGPL-3.0-or-later

//! `must-tui`: a terminal UI showing all must SEC nodes running for the user.
//!
//! It discovers the monitoring endpoints of all processes with must nodes
//! ([`discovery`]), keeps their state ([`app`]) and draws it ([`ui`]). It is read-only:
//! it sends endpoints nothing but its hello and traffic subscriptions.

pub mod app;
pub mod discovery;
pub mod terminal;
pub mod ui;
