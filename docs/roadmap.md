# Roadmap

MUST's features were planned as [OpenSpec](https://github.com/Bilchreis/MUST/tree/main/openspec)
changes and implemented in this order:

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
6. `add-status-tui` – `must-tui`, a terminal UI showing all running MUST nodes on the
   machine, opened by SHALL's status window flags (`must-tui`, done; manual check on
   Windows pending)
7. `sample-node-polling` – polling in two tiers inferred from the interface class, the
   `pollinterval` parameter and a poll hook for batched hardware access; the example
   node polls its noisy readback (`must-server`, done)
8. `add-udp-discovery` – SECoP UDP discovery of nodes on the network (`must-server`,
   done)
9. `concurrent-polled-requests` – polling-mode requests run concurrently, and running
   reads of one parameter are shared (`must-server`, `must-ffi`, done)
10. `user-errors-and-filtered-commands` – `filterStoredCommand` and SHALL's user error
    codes for SHALL-LabVIEW (`must-ffi`, done)
11. `add-matrix-datatype` – the SECoP 2.0 matrix data type (`must-types`, done)
12. `add-check-message` – answers to SECoP `check` requests (`must-server`, done)
13. `add-logging-messages` – SECoP remote logging (`must-server`, done)
14. `add-tui-secop-check` – the TUI's detail view checks a node's description with the
    SECoP checker (`secop-check`) and lists its findings (`must-tui`, done)

Planned, not started yet:

- `add-module-macro` – `#[must_server::module]` and `#[must_server::methods]`, which
  declare a module's parameters, commands and properties next to its behaviour
  (`must-server`)
- `add-node-config-files` – nodes built from TOML config files out of registered module
  types, with templates for many modules of one type (`must-config`)

SHALL's client library (`SECoP_C_*`) is out of scope.
