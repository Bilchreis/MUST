# must

must is a Rust rewrite of [SHALL](https://github.com/SampleEnvironment/SHALL), the
SECoP hardware abstraction layer. It lets existing programs, drivers and LabVIEW code
offer their hardware as a SEC node speaking [SECoP](https://github.com/SampleEnvironment/SECoP),
the Sample Environment Communication Protocol.

Differences to SHALL:

- written in Rust, without Qt (no Qt build step, library or runtime dependency)
- speaks SECoP 2.0 (draft) instead of SECoP V2019-09-16; the `matrix` data type and the
  optional `check`/`logging` messages are not supported yet
- a terminal UI (`must-tui`) replaces SHALL's Qt status window

## Status

Early development. The roadmap is planned as [OpenSpec](openspec/) changes, implemented
in this order:

1. `add-secop-data-types` – SECoP data types: datainfo, values, identifiers, parameter
   postfixes (crate `must-types`, done)
2. `add-sec-node-server` – SEC node library for Rust applications (`must-server`, done)
3. `add-shall-c-abi` – drop-in replacement for SHALL's C interface: libraries named
   `SHALLserver64`/`32` and `SHALLvariant64`/`32` exporting the same `SECoP_S_*` and
   `SECoP_V_*` functions, for C and LabVIEW callers (`must-ffi`, done; manual LabVIEW
   smoke test pending)
4. `add-node-monitoring` – per-process monitoring endpoint for status, clients, traffic
   and logs (`must-monitor`, `must_server::monitor`, done)
5. `add-node-log-files` – rolling per-node log files of all traffic and log records
   (`must_server::logfiles`, done)
6. `add-status-tui` – `must-tui`, a terminal UI showing all running must nodes on the
   machine, opened by SHALL's status window flags

SHALL's client library (`SECoP_C_*`) is out of scope.

## Platforms

| Target | Library files |
|---|---|
| Linux x86_64 | `lib*.so` |
| Windows x86_64 | `*.dll` |
| Windows i686 (32-bit, e.g. 32-bit LabVIEW) | `*.dll` |

CI builds and tests on all three.

## Crates

| Crate | Purpose |
|---|---|
| `crates/must-types` | SECoP 2.0 datainfo, values, identifiers and parameter postfixes |
| `crates/must-server` | SEC node library: declare a node, connect hardware handlers, serve it over TCP |
| `crates/must-monitor` | Monitoring protocol, per-user registry of endpoints, local transport and client library |
| `crates/must-ffi` | SHALL's C interface (`SECoP_S_*`, `SECoP_V_*`), built as `SHALLserver64`/`32` |
| `crates/must-ffi-variant` | `SHALLvariant64`/`32`, forwarding to `SHALLserver` |
| `crates/must-ffi-ctests` | C conformance tests of the C interface |

## Building

```bash
cargo test --workspace
```

A simulated temperature controller to try with any SECoP client (port 2055 by default):

```bash
cargo run --example simple_node
```

## Monitoring

Every process running must nodes offers a local, read-only monitoring endpoint (a Unix
domain socket on Linux, a named pipe on Windows, only accessible to the user) with the
status of its nodes, their clients, message traffic, parameter values and log records.
`must-tui` (planned) shows all of them; the protocol and the client library are described
in [docs/monitoring-protocol.md](docs/monitoring-protocol.md). Turn it off with
`MUST_MONITOR=0`.

## Log files

Every node writes what it exchanged with its clients (each published value once) and what
it logged to `~/.local/state/must/logs/<equipment_id>/<YYYY-MM-DD>.log` on Linux or
`%LOCALAPPDATA%\must\logs\<equipment_id>\<YYYY-MM-DD>.log` on Windows, and deletes files
older than 2 days. `MUST_LOG_FILES=0` turns it off, `MUST_LOG_DIR` and `MUST_LOG_DAYS`
change the directory and the retention:

```bash
MUST_LOG_DIR=/tmp/must-logs MUST_LOG_DAYS=7 cargo run --example simple_node
# writes /tmp/must-logs/must-example/<today>.log
```

The line format, disk usage and the Rust API are described in
[docs/log-files.md](docs/log-files.md).

## Replacing SHALL

Build the SHALL-named libraries and their headers into `dist/<target>/`:

```bash
cargo xtask dist                                 # this machine's target
cargo xtask dist --target i686-pc-windows-msvc   # 32-bit Windows, e.g. for 32-bit LabVIEW
cargo xtask check                                # exports, dependencies, cross-library calls
```

Then put `SHALLserver` and `SHALLvariant` in place of SHALL's files (both in one folder)
and remove SHALL's Qt libraries. C programs and LabVIEW VIs that use SHALL's C functions
run unchanged; see [COMPATIBILITY.md](COMPATIBILITY.md) for the details and the
differences to SHALL.

The toolchain version is pinned in `rust-toolchain.toml`; rustup installs it on first use.

## License

must is licensed under the GNU Lesser General Public License v3.0 or later
(LGPL-3.0-or-later), the same license as SHALL. See [COPYING.LESSER](COPYING.LESSER) and
[COPYING](COPYING).
