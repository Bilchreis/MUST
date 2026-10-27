# Status TUI (`must-tui`)

`must-tui` shows, in one terminal window, all must SEC nodes running for your user on
this machine: in Rust programs, C programs and LabVIEW, 32- or 64-bit. It replaces
SHALL's Qt status window. It only reads: nothing in it changes a node, a parameter or a
connection.

It reads the processes' monitoring endpoints (see
[monitoring-protocol.md](monitoring-protocol.md)), so monitoring must be on (the
default). Only one `must-tui` runs per user; a second one says so and exits.

## Starting it

- **By hand:** run `must-tui` in a terminal. It ships next to the libraries in
  `dist/<target>/`; `cargo run -p must-tui` runs it from the source tree.
- **From a program:** `SECoP_S_initLibrary` with `bGUI` set, or
  `SECoP_S_showStatusWindow(1)`, opens it in a window of its own, as SHALL's flags opened
  the status window. Rust programs call `must_server::show_status_tui()`. Nothing
  happens if a TUI already runs, and the call never fails or waits.
  - **Windows:** a new console window. Closing it ends only the TUI, never LabVIEW or
    your program.
  - **Linux:** a terminal emulator, when there is a graphical session (`DISPLAY` or
    `WAYLAND_DISPLAY`). The first one found is used: `MUST_TUI_TERMINAL`,
    `$TERMINAL -e`, `x-terminal-emulator`, `gnome-terminal`, `konsole`,
    `xfce4-terminal`, `xterm`. Without a graphical session or terminal emulator, an info
    log record says to run `must-tui` by hand.

  The TUI keeps running when the program that opened it ends, and it starts with that
  program's first node selected. `SECoP_S_showStatusWindow(0)` leaves it open, since it
  may show other programs' nodes too; close it with `q`.

  `must-tui` is looked for next to the must library (`SHALLserver64.dll` and so on, or
  the program for Rust), then at `MUST_TUI`, then on `PATH`. If it is not found, a
  warning log record names the places searched.

`must-tui --focus-pid <pid>` selects that process's first node once it appears.

## Views

| View | Shows |
|---|---|
| 1 Overview | every process (name, pid, must version) with its nodes: context ID, address (wide terminals), state (`building`, `failed`, `running`, `stopped`, `gone`), clients, messages per second, the most severe module status |
| 2 Detail | the selected node: state, address, context ID, the log file it writes (or that it writes none), build errors and warnings, every module with its status and every parameter's value (formatted with its unit), age or error, and the connected clients |
| 3 Traffic | the selected node's messages, live: time, `>` received or `<` sent, client, text; `…[cut]` marks messages shortened to 4 KiB. The node sends them only while this view is open. |
| 4 Logs | log records of all processes, merged by time; filters by level, node, process and text |

Statuses are shown as a colored label (`IDLE` green, `WARN` yellow, `BUSY` blue, `ERROR`
red, `DISABLED` grey) plus the status text, so nothing depends on the color. With
`NO_COLOR` set, no colors are used. In the classic Windows console, borders are drawn
with ASCII characters; in Windows Terminal and on Linux with line-drawing characters.

A process that ended is marked `GONE` and removed after 60 seconds, unless pinned with
`P`. A process is also marked when its endpoint dropped events because the TUI fell
behind (`lagged`), could not be reached, or speaks an unsupported protocol version.

## Keys

| Keys | Action |
|---|---|
| Tab / Shift+Tab | next / previous view |
| 1 2 3 4 | overview, detail, traffic, logs |
| Up / Down, j / k | select a node (overview), scroll (other views) |
| Left / Right | previous / next node |
| PgUp / PgDn | scroll by 10 lines |
| Home / End | oldest / newest lines |
| Enter | details of the selected node |
| Esc | back to the overview |
| / | filter (traffic, logs): Enter keeps it, Esc clears it |
| p | pause / resume the traffic view |
| l | log level: all, INFO+, WARN+, ERROR |
| n | logs of the selected node only, or of all |
| o | logs of the selected process only, or of all |
| P | pin the selected process (kept when it is gone) |
| ? | this help |
| q, Ctrl+C | quit |

`?` shows the same list in the TUI. Pausing the traffic view keeps receiving: the title
counts the new messages, and resuming shows them.

## Environment variables

| Variable | Meaning |
|---|---|
| `MUST_TUI` | path of `must-tui` (or the folder holding it), for programs that open the TUI |
| `MUST_TUI_TERMINAL` | Linux: terminal command to open the TUI in; the TUI's command line is appended, e.g. `alacritty -e` |
| `NO_COLOR` | no colors |
| `MUST_MONITOR_DIR` | registry directory of the monitoring endpoints (see [monitoring-protocol.md](monitoring-protocol.md)) |
