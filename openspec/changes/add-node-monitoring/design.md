# Design

## Context

`must-server` (change `add-sec-node-server`) emits a `NodeEvent` broadcast per node and structured `tracing` events. Host processes can be Rust programs, C programs or 32-bit/64-bit LabVIEW using the C ABI (`add-shall-c-abi`), and may run several nodes. The consumer is `must-tui` (`add-status-tui`), a separate process that may be 64-bit while the host is 32-bit. SHALL had no equivalent: its status window lived inside the host process.

## Goals / Non-Goals

**Goals:**
- Same mechanism on Linux and Windows, independent of process bitness.
- Zero impact on SEC node latency, bounded memory.
- Simple enough to debug by hand (readable JSON lines).

**Non-Goals:**
- Remote (cross-machine) monitoring.
- Controlling nodes (changing parameters, disconnecting clients) through the endpoint.
- Persisting logs to disk.

## Decisions

### Loopback TCP + token instead of local sockets
The endpoint is a TCP listener on `127.0.0.1` with an OS-assigned port. Access control comes from a 128-bit random token stored in the registration file, which only the user can read.
*Alternative:* Unix domain sockets / Windows named pipes (e.g. via the `interprocess` crate). Rejected: two transports to maintain, and tokio's TCP is already used by `must-server`. File permissions on the registration file give equivalent per-user protection for a read-only feed.

### Registry files rather than a fixed port or a broker
Each process writes `<pid>.json` into a per-user directory; consumers scan the directory (and watch it, see `add-status-tui`). No central daemon is needed and processes stay independent.
*Alternative:* a fixed well-known port with a broker process. Rejected: needs a daemon lifecycle and fails when two users share a machine.
Atomic write: write to `<pid>.json.tmp`, then rename. On Windows the file gets an ACL restricted to the current user (inherited from `%LOCALAPPDATA%`, which is already per-user).

### Stale detection
Registration includes the process start time (Linux: `/proc/<pid>/stat` field 22; Windows: `GetProcessTimes`) and process name. A registration is valid only if a process with that pid, start time and name exists. This handles crashed hosts and pid reuse. Readers delete stale files; writers overwrite their own pid's file.

### Protocol shape
Line-delimited JSON (`serde_json`, one object per line, `type` tag):
- client → endpoint: `hello {token, version, client}`; `subscribe_traffic {node, enabled}`.
- endpoint → client: `welcome {version, process}`, `snapshot {...}`, `event {kind, ...}`, `counters {node, rx, tx, per_second}`, `lagged {dropped}`, `error {message}`.
Version is `major.minor`; minor additions are optional fields that old readers ignore. The structure report is sent as embedded JSON, values in their SECoP JSON encoding, so the TUI can reuse `must-types` to interpret them.

### Event pipeline
A per-process `Monitor` singleton (created lazily by `must-server` when the first node starts) subscribes to every node's `NodeEvent` channel and installs a `tracing` layer for log capture. It fans events out to per-monitor bounded queues (capacity 4096); full queues drop oldest and count. Traffic texts are copied only when at least one monitor subscribed for that node, so unobserved nodes pay only a counter increment. Values come from the module caches for the snapshot, so a connecting monitor never triggers hardware reads.

### Tracing integration
`must-monitor` provides a `tracing_subscriber::Layer`. Rust hosts that already set a global subscriber add the layer themselves (documented); otherwise `must-server` installs a default subscriber with this layer when the monitor starts (C ABI hosts always get this path). Records keep `node` and `module` fields when the emitting span has them.

## Risks / Trade-offs

- [Another local process of the same user can read the token and the feed] → Same trust boundary as the user's own files; the feed is read-only. Documented.
- [Loopback TCP may be blocked by strict host firewalls on Windows] → Loopback is normally exempt; the error is logged and monitoring disabled for that process, nodes keep working.
- [Global `tracing` subscriber conflicts with a Rust host's own] → Never replace an existing global subscriber; document the layer for such hosts.
- [Large structure reports in snapshots] → Sent once per monitor connection; acceptable.

## Migration Plan

Not applicable: new functionality, on by default, off with `MUST_MONITOR=0`.
