// SPDX-License-Identifier: LGPL-3.0-or-later

//! The hook for SHALL's status window (`bGUI`, `SECoP_S_showStatusWindow`): opens the
//! status TUI `must-tui` in a window of its own (see [`must_server::launcher`]).

/// Requests that the status display is shown or hidden. Showing opens the TUI unless one
/// runs already; hiding leaves it open, since it may show other processes' nodes too.
pub(crate) fn show(visible: bool) {
    if visible {
        must_server::show_status_tui();
    }
}
