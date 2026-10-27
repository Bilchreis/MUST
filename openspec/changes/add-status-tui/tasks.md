# Tasks

## 1. TUI foundation

- [ ] 1.1 Add `crates/must-tui` (ratatui, crossterm, tokio, must-monitor, must-types) with app state, message loop, terminal setup/restore and panic hook; verify the panic-restores-terminal scenario manually and a unit test that the hook restores raw mode
- [ ] 1.2 Implement the discovery loop (2 s rescan, connect, gone marking, 60 s removal, pinning) against the `must-monitor` client; verify the new-node-appears and host-exits scenarios with an integration test that starts and stops host processes
- [ ] 1.3 Implement the single-instance lock on `tui.lock`; verify a second `must-tui` exits with a message while the first runs

## 2. Views

- [ ] 2.1 Implement the overview table grouped by process; verify the several-hosts scenario with a `TestBackend` snapshot test (insta)
- [ ] 2.2 Implement the node detail view with formatted values, status colors plus labels, errors and the clients pane; verify the busy-module and parameter-error scenarios with snapshot tests, also with `NO_COLOR`
- [ ] 2.3 Implement the traffic view with on-demand subscription, pause and filter; verify the live-traffic and pause scenarios
- [ ] 2.4 Implement the log view with level/process/node/text filters; verify the filter-errors-of-one-node scenario
- [ ] 2.5 Implement lag, connection and version error indicators; verify the incompatible-endpoint scenario with a fake endpoint
- [ ] 2.6 Implement key bindings and the help overlay; verify the help scenario and that no binding writes to nodes (read-only scenario)
- [ ] 2.7 Verify resize handling and 80×24 layout with snapshot tests at 80×24 and 120×40, and manually in Windows Terminal, conhost and a Linux terminal

## 3. Launching from the library

- [ ] 3.1 Implement executable lookup (library directory via `GetModuleHandleExW`/`dladdr`, `MUST_TUI`, `PATH`); verify the shipped-next-to-the-DLL scenario on Windows i686 and x86_64
- [ ] 3.2 Implement the Windows launcher with `CREATE_NEW_CONSOLE`; verify the close-the-console-window scenario manually and an automated test that the child has its own console and the host survives killing it
- [ ] 3.3 Implement the Linux launcher (graphical session check, terminal emulator order, `MUST_TUI_TERMINAL`, new session, null stdio); verify the no-graphical-session scenario in CI and a launch with `xterm` under Xvfb
- [ ] 3.4 Replace the no-op status display hook in `must-ffi` with the launcher (spawn thread, lock check, `--focus-pid`), add the Rust launch API, and implement hide as no-op; verify the first-init, already-running, missing-executable, hide-while-open and focus-on-launch scenarios

## 4. Distribution and docs

- [ ] 4.1 Add `must-tui` to `xtask dist` for all three targets; verify the dist directories contain the binary next to the libraries
- [ ] 4.2 Document the TUI (views, keys, launching, environment variables) in the README and update `COMPATIBILITY.md` (status window replaced by TUI, hide behavior); verify the documented key bindings match the help overlay
- [ ] 4.3 Run an end-to-end check on Windows: the C example host with `bGUI=1` opens the TUI showing its node, traffic and logs; record the result in the PR

## Workflow follow-up

- Archive after review.
