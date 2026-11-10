# Status TUI (`must-tui`)

`must-tui` shows, in one terminal window, all MUST SEC nodes running for your user on
this machine: in Rust programs, C programs and LabVIEW, 32- or 64-bit. It replaces
SHALL's Qt status window. It only reads: nothing in it changes a node, a parameter or a
connection.

It reads the processes' monitoring endpoints (see
[monitoring-protocol.md](monitoring-protocol.md)), so monitoring must be on (the
default). Only one `must-tui` runs per user; a second one says so and exits.

## Starting it

- **By hand:** run `must monitor` (or `must-tui`) in a terminal. `./install.sh`
  (`.\install.ps1` on Windows) installs both from a checkout into `~/.cargo/bin`, and
  `must update` installs the newest release (`vX.Y.Z` tag) in that checkout
  (`--tag v0.2.0` a given release, `--tag main` the latest commit on `main`),
  fetched from the public repository over HTTPS without credentials.
  `must` and `must-tui` also ship next to the libraries in `dist/<target>/` and the
  release archives (download new releases there; `must update` needs a checkout), and
  `cargo run -p must-tui` runs it from the source tree.
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

  `must-tui` is looked for next to the MUST library (`SHALLserver64.dll` and so on, or
  the program for Rust), then at `MUST_TUI`, then on `PATH`. If it is not found, a
  warning log record names the places searched.

`must-tui --focus-pid <pid>` selects that process's first node once it appears.

## Views

| View | Shows |
|---|---|
| 1 Overview | every process (name, pid, MUST version) with its nodes: context ID, address (wide terminals), state (`building`, `failed`, `running`, `stopped`, `gone`), clients, messages per second, the most severe module status |
| 2 Detail | the selected node: state, address, context ID, the log file it writes (or that it writes none), build errors and warnings, every module with its status and every parameter's value (formatted with its unit), age or error, the [SECoP check](#secop-check) of its description, and the connected clients |
| 3 Traffic | the selected node's messages, live: time, `>` received or `<` sent, client, text; `…[cut]` marks messages shortened to 4 KiB. The node sends them only while this view is open. Remote logging's `log` events are not shown: their records are in the Logs view. |
| 4 Logs | log records of all processes, merged by time: what the nodes log about themselves and what applications log for a module (`Ctx::log`, `SECoP_S_logMessage`) at error, warning or info; application debug records are not shown. Filters by level, node, process and text |

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
| c | check the node's description again (detail) |
| l | log level: all, INFO+, WARN+, ERROR |
| n | logs of the selected node only, or of all |
| o | logs of the selected process only, or of all |
| P | pin the selected process (kept when it is gone) |
| ? | this help |
| q, Ctrl+C | quit |

`?` shows the same list in the TUI. Pausing the traffic view keeps receiving: the title
counts the new messages, and resuming shows them.

## SECoP check

The detail view checks the selected node's description (its `describe` reply) against
the SECoP 2.0 specification with
[`secop-check`](https://github.com/SampleEnvironment/secop-checker) and lists what it
finds, most severe first: `fatal`, `error`, `warning`, `hint`, each with the module or
accessible it concerns. The section's title counts the findings, or says `ok`. A node is
checked when the detail view first shows it and again when its description changes;
`c` checks it again. The checker runs in the background on the description the TUI
already has: it does not connect to the node.

`secop-check` is a Python program. By default the TUI runs it with
[uv](https://docs.astral.sh/uv/) (`uvx`), which fetches it from its repository on first
use, so only uv needs to be installed. `MUST_SECOP_CHECK` names another command, e.g.
`secop-check` installed with `pipx` or `uv tool install`, or
`uv run --project /src/secop-checker secop-check` for a checkout; the TUI appends
`--json --version 2.0 -` and writes the description to its stdin. Without uv or the
command, the section says the check could not run.

## Environment variables

| Variable | Meaning |
|---|---|
| `MUST_TUI` | path of `must-tui` (or the folder holding it), for programs that open the TUI; `0` turns opening it off (e.g. on unattended machines whose programs set `bGUI`) |
| `MUST_TUI_TERMINAL` | Linux: terminal command to open the TUI in; the TUI's command line is appended, e.g. `alacritty -e` |
| `MUST_SECOP_CHECK` | the [SECoP checker](#secop-check) command, split at spaces; default `uvx --from git+https://github.com/SampleEnvironment/secop-checker@work secop-check` |
| `NO_COLOR` | no colors |
| `MUST_MONITOR_DIR` | registry directory of the monitoring endpoints (see [monitoring-protocol.md](monitoring-protocol.md)) |
