# Proposal

## Why

SHALL shows node status in a Qt window inside the host process. must replaces that with a separate TUI (`add-status-tui`) that shows all running must nodes on the machine, including several nodes inside one LabVIEW process and nodes in different processes. For that, every process hosting must nodes has to expose its nodes' status, client connections, traffic and logs in a way another process can find and read, on Linux and on Windows (32- and 64-bit hosts alike).

## What Changes

- Each process hosting at least one must node opens one local monitoring endpoint covering all its nodes, on loopback only.
- The endpoint is registered in a per-user runtime directory, so tools can discover every running must process; stale registrations are recognized and cleaned up.
- A versioned, line-delimited JSON monitoring protocol: on connect a snapshot (process, nodes, modules, parameter values, clients, recent logs), then a stream of events (nodes started/stopped, clients connected/disconnected, message traffic, parameter updates, log records).
- Log records of the library (and of the host application, if it logs through `tracing`) are captured in a bounded per-process buffer.
- Monitoring never slows down or blocks a SEC node; it can be turned off.
- A small Rust client library for discovering and reading endpoints, used by the TUI.

## Capabilities

### New Capabilities
- `node-monitoring-endpoint`: The per-process endpoint: what it publishes, when it exists, its protocol, access control and performance guarantees.
- `node-monitoring-discovery`: How endpoints are registered and found on a machine, including stale entries.

### Modified Capabilities
<!-- none -->

## Impact

- New crate `crates/must-monitor` (endpoint server, registry, client library); `must-server` starts/stops the endpoint with its nodes and feeds it from the `NodeEvent` stream and `tracing` (both defined in `add-sec-node-server`).
- The C ABI (`add-shall-c-abi`) gets monitoring automatically, since it uses `must-server`.
- New files at runtime: one registration file per process in the per-user runtime directory.
- Depends on `add-sec-node-server`.
