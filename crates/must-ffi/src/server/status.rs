// SPDX-License-Identifier: LGPL-3.0-or-later

//! The hook for SHALL's status window (`bGUI`, `SECoP_S_showStatusWindow`): opens the
//! status TUI `must-tui` in a window of its own and keeps it open (see
//! [`must_server::launcher`]).

/// Requests that the status display is shown or hidden. Showing opens the TUI unless one
/// runs already, and opens it again whenever it is closed; hiding stops that but leaves a
/// running TUI open, since it may show other processes' nodes.
pub(crate) fn show(visible: bool) {
    if visible {
        must_server::show_status_tui();
    } else {
        must_server::hide_status_tui();
    }
}
