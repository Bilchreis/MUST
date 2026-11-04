# Design

## Context

See proposal.md for the motivation and the specs for the required behavior.

**Today's code.**
- `Hub::log(level, module, message)` (`must-server/src/runtime/mod.rs`) is the single entry point for the node's log records. It writes to `tracing` (target `must_server::node`) and emits `NodeEvent::Log`, which feeds the monitor, the TUI and the log files. `LogLevel` has `Error`, `Warn` and `Info`. The module-scoped records are poll failures, failed reads and values that do not fit.
- `HubState::conns` holds one `ConnSink` per connection with its activation and an outgoing queue. `Hub::store` fans updates out under the hub lock. An update that would push a connection's counted bytes (`stats.updates`) over `max_outgoing_bytes` closes it.
- `protocol.rs` parses `logging` into `Request::Unsupported`. The parser test already expects `logging  "debug"` to have an empty specifier.
- The C ABI finds a running node by the `node:` part of a full name, as `SECoP_S_updateParameter` does.

## Goals / Non-Goals

**Goals:**
- Remote logging costs nothing when no connection asked for it: one check per record, no formatting.
- Applications get one way to log that reaches remote clients, the TUI and the log files.

**Non-Goals:**
- Logging per parameter. SECoP allows a node to widen it to the module, and must does.
- Forwarding node-wide records (module `None`). SECoP's `log` event needs a module name, and inventing one (such as the node's equipment id) would make clients treat it as a module.
- Adding a debug level to `LogLevel`, the monitor protocol or the log files.

## Decisions

### Levels live in `ConnSink`
Each `ConnSink` gets a `logging: LogFilter`, holding a node-wide level plus a map of module overrides. It is set by the connection's own `logging` request under the hub lock:
- A node-wide request sets the default and clears the overrides.
- A module request sets one override.

`HubState` also keeps a count of connections whose filter is not all-off. `Hub::log` checks that count first and builds the event line only when it is non-zero.
- *Alternative:* keep the levels in the connection task and broadcast every record to all tasks. Rejected: it needs a second fan-out channel, and it costs a send per record and connection even when nobody logs.

### Remote level is a separate, wider enum
`RemoteLevel { Error, Info, Debug }` is what a connection filters on. A record carries its `LogLevel` (`Error`/`Warn`/`Info`) or is a debug record. The mapping is Error/Warn → `error`, Info → `info`, Debug → `debug`. Warn maps to `error`: SECoP has no warning level, and the warnings must logs today (poll and read failures) are what a client at `"error"` wants to see.

Application debug records go through a new `Hub::log_debug(module, message)`, which writes to `tracing` at debug level and to remote clients only, never to `NodeEvent::Log`. This keeps chatty debug output out of the TUI's 2000-record buffer and the log files, which have no level filter of their own.

### Log events are best-effort
A log line is pushed only if the connection's counted bytes plus the line stay within `max_outgoing_bytes`. Otherwise it is dropped silently. Logging about the drop would feed the same queue. Log lines count toward the same byte counter as updates, so heavy logging can still make a later update close the connection. That is the existing slow-client rule, and it is documented.
- *Alternative:* a separate, smaller budget for log lines. Possible later; one limit is simpler to explain.

### Echo the level as received
The reply echoes the request's JSON value (`"debug"`, `false`, ...). must supports every level, so nothing needs widening except the parameter specifier, which is cut back to the module. A single-space `logging "info"` is recognized by its specifier starting with `"`, which no module name can.

### Application API
- **Rust:** `Ctx::log(level, message)` and `Publisher::log(level, message)` with `pub enum AppLogLevel { Error, Warning, Info, Debug }`. They call `Hub::log`/`log_debug` with the handler's module.
- **C:** `SECoP_S_logMessage(szModule, iLevel, szMessage)` resolves the node and module like `updateParameter` (`INVALID_NODE`/`INVALID_MODULE`), validates the level and text (`INVALID_VALUE`; the text must be UTF-8, with invalid bytes replaced), and calls the same functions. It takes no `szContextID`, like `SECoP_S_updateParameter[2]`, which also address nodes by full name. It never blocks on the node: `Hub::log` only takes the hub lock briefly.
- *Alternative:* name it `SECoP_S_log`. Rejected: too likely to collide with host symbols in a flat C namespace.

## Risks / Trade-offs

- [`Hub::log` runs under or near the hub lock and is called from update paths] → The fan-out builds the line once, and pushing is a queue append. As `Hub::store` already does for closed connections, `Hub::log` is never called while the hub lock is held. It takes the lock itself only to push the line.
- [A host logging in a tight loop at debug fills slow clients' buffers and gets them closed by the next update] → Documented next to `SECoP_S_logMessage`. Clients that ask for debug accept the volume.
- [Changes to the monitor or TUI are not needed, but application records now appear in the log files] → `docs/log-files.md` says so.
