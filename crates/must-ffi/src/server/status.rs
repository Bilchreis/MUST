// SPDX-License-Identifier: LGPL-3.0-or-later

//! The hook for SHALL's status window (`bGUI`, `SECoP_S_showStatusWindow`). It has no
//! effect until the status TUI (change `add-status-tui`) provides a display.

/// Requests that the status display is shown or hidden.
pub(crate) fn show(_visible: bool) {}
