# Tasks

## 1. Log file path through monitoring

- [x] 1.1 Record the current log file per node: the writer sets it in the node's hub and emits `NodeEvent::LogFile { path }` when it opens, switches or gives up a file; verify with writer unit tests (first file, midnight, pid fallback, failure clears it) and a runtime test seeing the event
- [x] 1.2 Protocol 1.1 in `must-monitor`: optional `log_file` in node info and a `log_file` event; the endpoint fills and forwards them; update `docs/monitoring-protocol.md`; verify the path-in-snapshot, day-change (with the writer's injected dates), logging-off and 1.0-reader scenarios

## 2. TUI foundation

- [x] 2.1 Add `crates/must-tui` (ratatui, crossterm, tokio, must-monitor, must-types) with app state, message loop, terminal setup/restore and panic hook; verify the panic-restores-terminal scenario manually and a unit test that the hook restores raw mode
- [x] 2.2 Implement the discovery loop (rescan every second, connect, gone marking, 60 s removal, pinning, `MUST_MONITOR_DIR`) against the `must-monitor` client; verify the new-node-appears and host-exits scenarios with an integration test that starts and stops host processes
- [ ] 2.3 Implement the single-instance lock on `tui.lock` (`File::try_lock` on a read/write handle); verify a second `must-tui` exits with a message while the first runs, on all three targets

## 3. Views

- [x] 3.1 Implement the overview table grouped by process, with context IDs and building/failed states; verify the several-hosts and node-being-built-and-failed-node scenarios with `TestBackend` snapshot tests (insta)
- [x] 3.2 Implement the node detail view with formatted values, status colors plus labels, errors, the clients pane, build messages and the log file path; verify the busy-module, parameter-error, failed-node, log-file and file-logging-off scenarios with snapshot tests, also with `NO_COLOR`
- [x] 3.3 Implement the traffic view with on-demand subscription, pause, filter and the mark for shortened messages; verify the live-traffic and pause scenarios
- [x] 3.4 Implement the log view with level/process/node/text filters; verify the filter-errors-of-one-node scenario
- [x] 3.5 Implement lag, connection and version error indicators; verify the incompatible-endpoint scenario with a fake endpoint
- [x] 3.6 Implement key bindings and the help overlay; verify the help scenario, and the read-only scenario with a test that records everything the TUI sends to a fake endpoint (only hello and subscriptions)
- [ ] 3.7 Verify resize handling and the 80×24 layout with snapshot tests at 80×24 and 120×40, and manually in Windows Terminal, conhost and a Linux terminal

## 4. Launching from the library

- [ ] 4.1 Implement the module-directory lookup in `must-monitor` (Windows `GetModuleHandleExW`/`GetModuleFileNameW`, Linux `/proc/self/maps`) and the executable lookup (library directory, `MUST_TUI`, `PATH`); verify the shipped-next-to-the-DLL scenario on Windows i686 and x86_64 and a Linux test from the built `SHALLserver64` library
- [ ] 4.2 Implement the Windows launcher (`CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP`, null stdio); verify the close-the-console-window scenario manually, and an automated test that the child has its own console and the host survives killing it
- [x] 4.3 Implement the Linux launcher (graphical session check, `MUST_TUI_TERMINAL`, terminal emulator order, own process group, null stdio, reaping); verify the no-graphical-session scenario in CI and a launch with `xterm` under Xvfb
- [x] 4.4 Add `must_server::show_status_tui()` (spawn thread, lock check, `--focus-pid`) and replace the no-op status display hook in `must-ffi` with it, hide as no-op; verify the first-init, already-running, missing-executable, hide-while-open and focus-on-launch scenarios

## 5. Distribution and docs

- [ ] 5.1 Add `must-tui` to `xtask dist` for all three targets; verify the dist directories contain the binary next to the libraries
- [x] 5.2 Document the TUI (views, keys, launching, environment variables) in the README and `docs/`, and update `COMPATIBILITY.md` (status window replaced by the TUI, hide behavior); verify the documented key bindings match the help overlay
- [ ] 5.3 Run an end-to-end check on Windows: the C example host with `bGUI=1` opens the TUI showing its node, traffic, logs and log file path; record the result in the PR

## Workflow follow-up

- Archive after review.
