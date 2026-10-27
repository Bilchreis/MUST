# Proposal

## Why

SHALL users get a Qt status window (`bGUI` in `SECoP_S_initLibrary`, `SECoP_S_showStatusWindow`) that shows their nodes, connections and messages. must has no Qt, and a library loaded into LabVIEW has no terminal of its own. With per-process monitoring endpoints in place (`add-node-monitoring`), a separate terminal UI can show every running must node on the machine in one place, and the library can open it in its own window when SHALL's GUI flags ask for it.

## What Changes

- New binary `must-tui` (ratatui + crossterm) that discovers all monitoring endpoints of the current user and shows every running must node across all processes: overview, node detail (modules, parameters, status, clients), message traffic and logs.
- It follows processes appearing and disappearing and copes with lagging or vanished endpoints.
- Read-only: it never changes node state.
- The C ABI's status display hook (left as a no-op in `add-shall-c-abi`) now launches `must-tui` in a new terminal window when `bGUI` is set or `SECoP_S_showStatusWindow(1)` is called: a new console window on Windows, a terminal emulator on Linux when a graphical session is available. Only one TUI runs per user; if one is already running, no second one is started.
- `must-tui` is shipped next to the SHALL-named libraries in the distribution.

## Capabilities

### New Capabilities
- `status-tui`: The terminal UI: discovery, views, navigation, filtering, and behavior when processes come and go.
- `status-tui-launch`: How the library starts the TUI in its own terminal window on Windows and Linux, finds the executable, and avoids duplicates.

### Modified Capabilities
<!-- none -->

## Impact

- New crate `crates/must-tui` (binary); depends on `must-monitor` (client library) and `must-types` (formatting values).
- New launcher module in `must-ffi` replacing the no-op status display hook; also usable from Rust (`must_server::show_status_tui()`).
- Distribution (`xtask dist`) includes `must-tui` / `must-tui.exe` per target.
- Depends on `add-node-monitoring` and `add-shall-c-abi`.
