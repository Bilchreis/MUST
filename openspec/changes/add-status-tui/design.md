# Design

## Context

`add-node-monitoring` gives each host process one local endpoint (Unix domain socket on Linux, named pipe on Windows) with a registration file in a per-user directory, plus a Rust client library (`must-monitor`) that lists endpoints and streams snapshots and events. `add-shall-c-abi` routes `bGUI` and `SECoP_S_showStatusWindow` to a no-op status display hook. Hosts include 32-bit LabVIEW on Windows (a GUI process without a console) and console or GUI programs on Linux.

## Goals / Non-Goals

**Goals:**
- One TUI per user showing all must nodes on the machine.
- Opening the TUI from the C ABI must never endanger the host process.
- The same TUI binary works whichever bitness the hosts have.

**Non-Goals:**
- Writing parameters or controlling nodes from the TUI.
- Monitoring other machines.
- Drawing a UI inside the host process (no `AllocConsole`, no embedded mode).

## Decisions

### Stack
`ratatui` with the `crossterm` backend (works on Windows consoles and Unix terminals), `tokio` for endpoint connections, `must-monitor` for discovery and protocol, `must-types` to format values (fmtstr, units, enum names). One app state (`App`) is updated from a single channel of `Msg` (input events, monitor events, ticks); rendering is a pure function of `App`, which makes views testable with ratatui's `TestBackend`.

### Discovery loop
Every 2 s list endpoints; connect to new ones; keep a `Process` entry per registration (pid + start time). Endpoint stream end or a stale listing marks the process gone; gone processes are removed after 60 s unless pinned. File-system watching is not used: polling a small directory every 2 s is cheap and portable.

### Views
- Overview: table grouped by process: node, address, state, clients, msg/s, worst status.
- Node detail: modules (status color by code group: 0 DISABLED grey, 1xx IDLE green, 2xx WARN yellow, 3xx BUSY blue, 4xx ERROR red) with parameters (formatted value, unit, age or error), and a clients pane.
- Traffic: ring buffer of 10 000 messages per node while subscribed; pause keeps buffering; `/` filter.
- Logs: merged ring buffer of all processes' records (10 000), filters for level, process, node, text.
Status colors are paired with text labels so `NO_COLOR` and color-blind users lose no information.

### Single instance
The TUI holds an exclusive lock on `tui.lock` in the registry directory (`fs4`/`fd-lock`: `flock` on Linux, `LockFileEx` on Windows) for its lifetime. The launcher tries to take the same lock non-blockingly: if that fails, a TUI is running and nothing is started. The OS releases the lock when the TUI dies, so there are no stale locks.
*Alternative:* pid file. Rejected: stale-pid handling again; locks clean themselves up.

### Launching on Windows
`CreateProcessW` with `CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP` and the TUI path; the child gets its own console window, so `CTRL_CLOSE_EVENT` from closing it reaches only the TUI. The host's handles are not inherited. The library locates itself with `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, ...)` + `GetModuleFileNameW` to search next to the DLL first.
*Alternative:* `AllocConsole` in the host and draw there. Rejected: closing that console window terminates the host (e.g. LabVIEW) and a process can have only one console.

### Launching on Linux
If `DISPLAY` or `WAYLAND_DISPLAY` is set: try `$TERMINAL -e`, `x-terminal-emulator -e`, `gnome-terminal --`, `konsole -e`, `xfce4-terminal -x`, `xterm -e` (first found on `PATH`), start with `std::process::Command` in a new session (`setsid` via `process_group(0)`/`pre_exec`) and stdin/stdout/stderr set to null, so the TUI outlives the host and gets no signals from its terminal. The library's own path comes from `dladdr` on a function address.

### Launch call path
The hook spawns on a dedicated short-lived thread and returns immediately; failures are logged via `tracing` (which the monitor forwards). `--focus-pid <pid>` tells the TUI which process to select first.

### Distribution
`xtask dist` adds `must-tui[.exe]` built for the same target triple. A 32-bit TUI beside 32-bit DLLs is fine since the monitoring protocol is bitness-neutral.

## Risks / Trade-offs

- [No terminal emulator on minimal Linux desktops] → Log an instruction to run `must-tui` manually; `MUST_TUI_TERMINAL` env var can name a custom terminal command.
- [Some terminals need different argument syntax] → Known emulators get their correct flag; `$TERMINAL` is used with `-e`, the de-facto convention.
- [conhost has limited colors and Unicode] → Use only 16 base colors and ASCII-safe borders by default; detect Windows Terminal (`WT_SESSION`) for richer symbols.
- [SHALL's `SECoP_S_showStatusWindow(0)` closed the window; must leaves it open] → Documented in `COMPATIBILITY.md`; the TUI is shared across processes, so one host must not close it for others.
