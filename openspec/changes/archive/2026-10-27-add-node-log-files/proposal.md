# Proposal

## Why

When a measurement goes wrong, users need to see afterwards what a SEC node exchanged with its clients and what it reported, for example after a LabVIEW session has ended. Today must only logs to `tracing` (by default nowhere), and `add-node-monitoring` keeps logs only in memory while the process runs. A rolling on-disk log per node, on by default, gives that history without anyone having had a monitor open.

## What Changes

- Every node writes a log file per day into a per-user log directory, one folder per node (named after its `equipment_id`).
- The files contain every SECoP message the node received and sent on the wire, plus the library's own log records for that node (client connects and disconnects, warnings, errors), in time order, as readable text lines.
- Files older than the retention are deleted; the retention is 2 days by default and configurable. There is no size limit.
- On by default; directory and retention configurable, and file logging can be turned off, through environment variables (for every host, including C and LabVIEW programs using the SHALL C ABI) and the Rust API.
- Logging to disk never blocks or slows a node; if the disk cannot keep up or is unavailable, lines are dropped (and the gap is marked) or logging is disabled with a warning, and the node keeps working.

## Capabilities

### New Capabilities
- `node-log-files`: Per-node rolling log files on disk: what is logged, file layout and naming, retention, configuration, and the guarantee that file logging never affects a node.

### Modified Capabilities
<!-- none: nodes' protocol and runtime behavior are unchanged -->

## Impact

- `must-server`: a file logger fed from each node's `NodeEvent` stream and from the node's own `tracing` records (which need the node as context); configuration on `NodeBuilder`.
- C ABI (`SHALLserver`): gets file logging automatically; configured only through environment variables, since SHALL's exported functions must stay unchanged.
- New files at runtime: `<log dir>/<equipment_id>/<YYYY-MM-DD>.log`, by default under `~/.local/state/must/logs` (Linux) and `%LOCALAPPDATA%\must\logs` (Windows).
- Independent of `add-node-monitoring` (in-memory, live view); both read the same node events.
- Depends on `add-sec-node-server` (archived).
