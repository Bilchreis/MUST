# Design

## Context

`add-node-monitoring` gives each host process one local endpoint (Unix domain socket on Linux, named pipe on Windows) with a registration file in a per-user directory, plus a Rust client library (`must-monitor`) that lists endpoints and streams snapshots and events. The protocol (1.0) carries:
- node states (`building`, `failed`, `running`, `stopped`), SHALL context IDs and build messages
- structure reports and the cached parameter reports
- clients, and traffic per node on request
- coalesced or all updates, per-second counters
- log records, with a `lagged` notice when a monitor falls behind

`add-node-log-files` writes each node's log to `<log dir>/<node folder>/<date>[-<pid>].log`; only the writer thread knows which file is in use. `add-shall-c-abi` routes `bGUI` and `SECoP_S_showStatusWindow` to a no-op status display hook (`must-ffi`, `server/status.rs`). Hosts include 32-bit LabVIEW on Windows (a GUI process without a console) and console or GUI programs on Linux.

## Goals / Non-Goals

**Goals:**
- One TUI per user showing all must nodes on the machine, including nodes being built or failed.
- Opening the TUI from the C ABI must never endanger the host process.
- The same TUI binary works whichever bitness the hosts have.

**Non-Goals:**
- Writing parameters, calling commands or controlling nodes from the TUI (read-only, as SHALL's status window).
- Showing the content of log files (the TUI shows where they are).
- Monitoring other machines.
- Drawing a UI inside the host process (no `AllocConsole`, no embedded mode).

## Decisions

### Stack
- `ratatui` with the `crossterm` backend (works on Windows consoles and Unix terminals), `tokio` for endpoint connections.
- `must-monitor` for discovery and protocol, `must-types` to format values (fmtstr, units, enum names).
- One app state (`App`) is updated from a single channel of `Msg` (input events, monitor messages, ticks).
- Rendering is a pure function of `App`, which makes views testable with ratatui's `TestBackend` and `insta` snapshots.

### Discovery loop
Every second the TUI lists endpoints (the spec asks for at least every 2 s), connects to new ones, and keeps a `Process` entry per registration (pid + start time). An endpoint stream ending, or a listing that no longer shows the process, marks the process gone; gone processes are removed after 60 s unless pinned. File-system watching is not used: polling a small directory every 2 s is cheap and portable. `MUST_MONITOR_DIR` is honored as by every reader.

### Views
- **Overview:** a table grouped by process (name, pid, context IDs). Per node:
  - equipment_id, address, state
  - clients, msg/s from `counters`
  - worst module status
- **Node detail:**
  - Modules, with the status colored by code group: 0 DISABLED grey, 1xx IDLE green, 2xx WARN yellow, 3xx BUSY blue, 4xx ERROR red.
  - Parameters: formatted value, unit, age or error.
  - A clients pane.
  - The build messages (errors when failed, warnings).
  - The current log file path, or "file logging off".
- **Traffic:** a ring buffer of 10 000 messages per node while subscribed (`subscribe_traffic` only while the view is open). Pause keeps buffering; `/` filters. Texts the endpoint cut at 4 KiB are marked `…[cut]`.
- **Logs:** the endpoint snapshots' records plus live ones, merged by time in one ring buffer (10 000), with filters for level, process, node and text.

Status colors are paired with text labels, so `NO_COLOR` and color-blind users lose no information. The TUI sends only `hello`, `subscribe_traffic` and `subscribe_updates`: nothing it can send changes a node.

### Log file path through monitoring (protocol 1.1)
- The writer thread is the only place that knows the file in use, since it handles the day change and the fallback to `<date>-<pid>.log`.
- When it opens, switches or gives up a node's file, it records the path in the node's hub (a `Mutex<Option<PathBuf>>`) and emits a new `NodeEvent::LogFile { path: Option<PathBuf> }`. It holds a `Weak<Hub>` already, and the event emit never blocks.
- The monitor puts the current path into node info as `log_file`, absent when logging is off or stopped, and forwards changes as a `log_file` event.
- Version 1.1 only adds optional fields and an event kind, which 1.0 readers ignore.

### Single instance
- The TUI holds an exclusive lock on `tui.lock` in the registry directory for its lifetime, using `std::fs::File::try_lock` on a file opened read/write (Windows `LockFileEx` needs read or write access, not append).
- The launcher tries the same lock non-blockingly and releases it at once. If that fails, a TUI is running and nothing is started.
- The OS releases the lock when the TUI dies, so there are no stale locks. On Windows the lock is mandatory, but nothing else reads this file.

*Alternative:* a pid file. Rejected: stale pids would need handling again; locks clean themselves up.

### Launcher location
The launcher lives in `must-server` (`launcher.rs`, `show_status_tui()`), so Rust hosts and the C ABI share it; `must-ffi`'s status hook calls it. Finding the directory of the module that contains the launcher is a platform function in `must-monitor`, which already holds the crate's only unsafe platform code:
- **Windows:** `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | ..._UNCHANGED_REFCOUNT)` and `GetModuleFileNameW`.
- **Linux:** the mapping in `/proc/self/maps` that contains the address. No libc call needed.

### Launching on Windows
- `CreateProcessW` (in `must-monitor`'s platform code) with `CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP`, no inherited handles, and no `STARTF_USESTDHANDLES`. The child uses its new console's own handles, and `CTRL_CLOSE_EVENT` from closing that window reaches only the TUI.
- `std::process::Command` is not used here: it always passes the parent's standard handles. From a console host, the TUI would then draw into the host's console; from LabVIEW, it would get no console handles.
- The child is not waited for (no zombie on Windows).

*Alternative:* `AllocConsole` in the host and draw there. Rejected: closing that console window terminates the host (e.g. LabVIEW), and a process can have only one console.

### Launching on Linux
- Only if `DISPLAY` or `WAYLAND_DISPLAY` is set.
- The terminal command comes from `MUST_TUI_TERMINAL` (a command line, the TUI arguments appended), else the first found of:
  - `$TERMINAL -e`
  - `x-terminal-emulator -e`
  - `gnome-terminal --`
  - `konsole -e`
  - `xfce4-terminal -x`
  - `xterm -e`
- Started with `std::process::Command`, `process_group(0)` and null stdio, so the TUI is not hit by signals to the host's process group.
- A detached thread waits for the terminal process to reap it.

### Launch call path
The hook spawns on a dedicated short-lived thread and returns immediately. Failures are logged via `tracing`, which the monitor forwards. `--focus-pid <pid>` tells the TUI which process to select first.

### Distribution
`xtask dist` adds `must-tui[.exe]` built for the same target triple. A 32-bit TUI beside 32-bit DLLs is fine, since the monitoring protocol is bitness-neutral.

## Risks / Trade-offs

- [No terminal emulator on minimal Linux desktops] → Log an instruction to run `must-tui` manually; `MUST_TUI_TERMINAL` can name a custom terminal command.
- [Some terminals need different argument syntax] → Known emulators get their correct flag; `$TERMINAL` is used with `-e`, the de-facto convention.
- [conhost has limited colors and Unicode] → Use only the 16 base colors and ASCII-safe borders by default; detect Windows Terminal (`WT_SESSION`) for richer symbols.
- [SHALL's `SECoP_S_showStatusWindow(0)` closed the window; must leaves it open] → Documented in `COMPATIBILITY.md`; the TUI is shared across processes, so one host must not close it for others.
- [A log file path can be long] → The detail view shortens the middle with `…` to fit and shows the full path in a footer when selected.
