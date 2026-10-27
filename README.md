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
2. `add-sec-node-server` – SEC node library for Rust applications (`must-server`, in
   progress)
3. `add-shall-c-abi` – drop-in replacement for SHALL's C interface: libraries named
   `SHALLserver64`/`32` and `SHALLvariant64`/`32` exporting the same `SECoP_S_*` and
   `SECoP_V_*` functions, for C and LabVIEW callers
4. `add-node-monitoring` – per-process monitoring endpoint for status, clients, traffic
   and logs
5. `add-status-tui` – `must-tui`, a terminal UI showing all running must nodes on the
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

## Building

```bash
cargo test --workspace
```

A simulated temperature controller to try with any SECoP client (port 2055 by default):

```bash
cargo run --example simple_node
```

The toolchain version is pinned in `rust-toolchain.toml`; rustup installs it on first use.

## License

must is licensed under the GNU Lesser General Public License v3.0 or later
(LGPL-3.0-or-later), the same license as SHALL. See [COPYING.LESSER](COPYING.LESSER) and
[COPYING](COPYING).
