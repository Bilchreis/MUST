# MUST

MUST is a Rust rewrite of [SHALL](https://github.com/SampleEnvironment/SHALL), the
SECoP hardware abstraction layer. It lets existing programs, drivers and LabVIEW code
offer their hardware as a SEC node speaking [SECoP](https://github.com/SampleEnvironment/SECoP),
the Sample Environment Communication Protocol. Installation and a first overview are in
the [README](https://github.com/Bilchreis/MUST#readme).

This guide describes monitoring, the status TUI, log files, polling and discovery, the
roadmap, and how MUST is tested and released.

## API docs

The API docs of the Rust crates, built with `cargo doc`:

- [`must-types`](api/must_types/index.html): SECoP 2.0 data types
- [`must-server`](api/must_server/index.html): the SEC node library for Rust programs
- [`must-monitor`](api/must_monitor/index.html): the monitoring endpoint and its protocol
- [`must-tui`](api/must_tui/index.html): the status TUI
- [`must-ffi`](api/must_ffi/index.html): the SHALL-compatible C ABI (`SECoP_S_*`, `SECoP_V_*`)
