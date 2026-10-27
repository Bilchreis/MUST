# Proposal

## Why

`must-types` (change `add-secop-data-types`) can describe and validate SECoP data, but nothing serves it yet. The core of SHALL is the SEC node: an application declares modules with parameters and commands, connects them to its hardware code, and the library speaks SECoP to any number of clients over TCP. must needs this as a Rust library, speaking SECoP 2.0, before the SHALL-compatible C ABI (`add-shall-c-abi`) can be layered on top.

## What Changes

- New crate `must-server` providing a SEC node library for Rust applications.
- Build a node in code: node, module and accessible properties (all mandatory and optional SECoP 2.0 properties, plus custom `_` properties), checked for completeness and consistency before the node can start, including interface-class rules and the module-level parameter postfix rules.
- Generate the SECoP 2.0 structure report for `describe` from the model.
- Serve the node over TCP (chosen interface and port, many simultaneous clients) with the mandatory SECoP 2.0 messages: `*IDN?`, `describe`, `activate`/`deactivate` (also module-wise), `ping`, `read`, `change`, `do`, `update`/`error_update`, and `error_*` replies with SECoP error reports.
- The optional `check` and `logging` messages are not implemented and answered with `ProtocolError`.
- Connect hardware code through an async handler per module (read, change, do), with a value cache, application-pushed updates (like `SECoP_S_updateParameter`), optional periodic polling, timeouts, and the SECoP ordering rules (side-effect updates before `changed`/`done`, initial updates before `active`).
- Run several independent nodes in one process; start and stop them cleanly.
- Add the complete SECoP error class list to `must-types`.

## Capabilities

### New Capabilities
- `sec-node-model`: Declaring a SEC node with its modules, parameters, commands and properties, validating it, and producing its structure report.
- `sec-node-protocol`: The SECoP 2.0 wire protocol of a node: message framing and parsing, the mandatory request/reply/event messages, and error replies.
- `sec-node-runtime`: How a running node reaches the hardware: handlers, value cache, updates to activated clients, polling, timeouts, ordering guarantees, connections and node lifecycle.

### Modified Capabilities
<!-- none: the capabilities of add-secop-data-types are used, not changed -->

## Impact

- New crate `crates/must-server`; depends on `must-types`.
- New dependencies: `tokio` (TCP, timers, tasks), `tracing` (structured logs, later consumed by `add-node-monitoring`).
- `must-types` gains the full `ErrorClass` enum and data-report/error-report encoding.
- On the wire must speaks SECoP 2.0 (`*IDN?` → `ISSE,SECoP,<draft-date>,v2.0`), unlike SHALL's V2019-09-16; SHALL users moving to must will see this difference in their clients.
