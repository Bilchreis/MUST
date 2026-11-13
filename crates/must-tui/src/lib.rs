// SPDX-License-Identifier: LGPL-3.0-or-later

//! `must-tui`: a terminal UI showing all MUST SEC nodes running for the user.
//!
//! It discovers the monitoring endpoints of all processes with MUST nodes
//! ([`discovery`]), keeps their state ([`app`]) and draws it ([`ui`]). It is read-only:
//! it sends endpoints nothing but its hello and traffic subscriptions. The detail view
//! checks the selected node's structure report with the SECoP checker ([`checker`]).

pub mod app;
pub mod checker;
pub mod discovery;
pub mod format;
pub mod terminal;
pub mod ui;
