# Proposal

## Why

SHALL users get a Qt status window (`bGUI` in `SECoP_S_initLibrary`, `SECoP_S_showStatusWindow`) that shows their nodes, connections and messages. must has no Qt, and a library loaded into LabVIEW has no terminal of its own. With per-process monitoring endpoints in place (`add-node-monitoring`), a separate terminal UI can show every running must node on the machine in one place, and the library can open it in its own window when SHALL's GUI flags ask for it.

## What Changes

- New binary `must-tui` (ratatui + crossterm) that discovers all monitoring endpoints of the current user and shows every must node across all processes:
  - overview, including nodes still being built or failed, and their SHALL context IDs
  - node detail: modules, parameters, status, clients, build messages, and the path of the node's current log file
  - message traffic
  - logs
- It follows processes appearing and disappearing and copes with lagging or vanished endpoints.
- Read-only: it never changes node state.
- The monitoring endpoint reports each node's current log file (from `add-node-log-files`): an optional field in the snapshot and an event when the file changes. This is protocol version 1.1, an addition that 1.0 readers ignore.
- The C ABI's status display hook (left as a no-op in `add-shall-c-abi`) now launches `must-tui` in a new terminal window when `bGUI` is set or `SECoP_S_showStatusWindow(1)` is called:
  - a new console window on Windows
  - a terminal emulator on Linux, when a graphical session is available
  - Only one TUI runs per user; if one is already running, no second one is started.
  - The host keeps the TUI open: when it is closed, it is opened again, until `SECoP_S_showStatusWindow(0)` or the library is done.
- Rust hosts get the same launch through `must_server::show_status_tui()`.
- `must-tui` is shipped next to the SHALL-named libraries in the distribution.

## Capabilities

### New Capabilities
- `status-tui`: The terminal UI: discovery, views, navigation, filtering, and behavior when processes come and go.
- `status-tui-launch`: How the library starts the TUI in its own terminal window on Windows and Linux, finds the executable, and avoids duplicates.

### Modified Capabilities
- `node-monitoring-endpoint`: Reports the path of each node's current log file.

## Impact

- New crate `crates/must-tui` (binary); depends on `must-monitor` (client library) and `must-types` (formatting values).
- `must-monitor`:
  - protocol 1.1, with the optional `log_file` in node info and a `log_file` event
  - a platform function that finds the directory of the module (DLL/shared object) containing a given code address
- `must-server`:
  - the writer reports the current log file to the node
  - the monitor forwards it
  - a new launcher module (`show_status_tui()`)
- `must-ffi`: the status display hook calls the launcher.
- Distribution (`xtask dist`) includes `must-tui` / `must-tui.exe` per target.
- Depends on `add-node-monitoring`, `add-node-log-files` and `add-shall-c-abi`.
