# Proposal

## Why

SECoP 2.0 defines remote logging. A client sends `logging [<module>] <level>`, and the node then sends that client `log <module>:<level> <message>` events for the module. An ECS can then show why a module misbehaves without access to the node's machine. must answers `logging` with a `ProtocolError` today. Its log records (poll failures, values that do not fit their datainfo) only reach the status TUI and the log files, both on the node's machine. Applications have no way to log anything themselves: neither Rust handlers nor C/LabVIEW hosts.

## What Changes

- **must-server answers `logging`**:
  - The request is `logging <module>[:<parameter>] <level>`, or `logging  <level>` for the whole node; `logging <level>` with one space is also accepted.
  - `<level>` is `"debug"`, `"info"`, `"error"` or `"off"`. `false` is accepted as `"off"`, as in SECoP's own example.
  - The node keeps a level per connection and module, `"off"` by default. A node-wide request sets all modules. A parameter specifier applies to its module, and the reply names only the module.
  - The reply mirrors the request with the specifier actually used.
  - Unknown modules get `NoSuchModule`, invalid levels `WrongType`.
- **`log` events**: each module-scoped log record whose level passes a connection's level for that module is sent to that connection as `log <module>:<level> "<message>"`.
  - must's levels map to SECoP's: error and warning → `error`, info → `info`, debug → `debug`.
  - Log events never cause a disconnect: while a connection's outgoing buffer is over its limit, its log events are dropped. Node-wide records without a module are not sent, since the event needs a module name.
- **Applications can log**:
  - Rust handlers: `Ctx::log` and `Publisher::log`, with levels error, warning, info and debug.
  - C hosts: new export `SECoP_S_logMessage(szModule, iLevel, szMessage)`, a must addition, with new enum `SECoP_S_logLevel`. `szModule` is `node:module`.
  - Error, warning and info records also reach the status TUI and the log files, like must's own records. Debug records go only to remote clients and `tracing`, so a chatty host does not flood the log files.
- **Headers and docs**: `SECoP-exports.h` and `SECoP-types.h` declare the addition. `COMPATIBILITY.md` lists remote logging and the new function.

No breaking change. Clients that sent `logging` got `ProtocolError` before and now get the documented reply.

## Capabilities

### New Capabilities
- `sec-node-remote-logging`: the `logging` request, per-connection levels, `log` events, and how application and node log records become events.

### Modified Capabilities
- `sec-node-protocol`: removes "Logging not supported" (added by `add-check-message`).
- `shall-server-abi`: new requirement "Application log messages".

## Impact

- `crates/must-server`:
  - `protocol.rs`: parse `logging`.
  - `runtime/connection.rs`: per-connection levels; replies.
  - `runtime/mod.rs`: `Hub::log` fans module records out to connections; debug records.
  - `handler.rs`: `Ctx::log`, `Publisher::log`.
  - Outgoing queue: droppable log events.
- `crates/must-ffi`: `server/api.rs` gets `SECoP_S_logMessage`.
- `include/SECoP-exports.h`, `include/SECoP-types.h`: declarations.
- `crates/must-ffi-ctests`: header checks, C tests.
- Docs: `COMPATIBILITY.md`, `must-server` crate docs, `docs/log-files.md` (application records appear there; debug ones do not).
- Related: depends on `add-check-message`, which splits the old "Optional and custom messages" requirement. Archive that change first.
