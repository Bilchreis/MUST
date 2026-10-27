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

### OS-local connections, behind a transport seam
The endpoint is a Unix domain socket `<registry dir>/<pid>.sock` on Linux (the directory is 0700, the socket 0600) and a named pipe `\\.\pipe\must-<user SID>-<pid>` on Windows, both with tokio (`tokio::net::UnixListener`, `tokio::net::windows::named_pipe`). The operating system enforces that only the user's processes can connect, so no token is needed, and nothing listens on a network port: no firewall prompts, no exposure.
On Windows the default security descriptor of a named pipe grants read access to Everyone, so the pipe is created with an explicit descriptor that allows only the current user's SID (and SYSTEM). The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so no other process can have taken the name, and the client checks `GetNamedPipeServerProcessId` against the registration's pid before trusting the stream.
Both sides use one small seam: the endpoint opens a `Listener` that yields byte streams, and the client library has one `connect(&EndpointDescription)` that returns a byte stream. Everything above (hello, snapshot, events, coalescing) only sees `AsyncRead + AsyncWrite`, and the TUI only calls the client library. Adding local TCP later is a new `kind` (`tcp`, with address and token), one listener and one connector, with no protocol or TUI change; readers that do not know a kind skip it (spec "Endpoint description").
*Alternative:* local TCP on `127.0.0.1` with a random port and a token in the registration file. Deferred: one code path for both platforms, but a network port that firewalls may question and a token that any process of the user can read. The seam keeps it available.

### Registry files rather than fixed names or a broker
Each process writes `<pid>.json` into a per-user directory; consumers scan the directory (and watch it, see `add-status-tui`). No central daemon is needed and processes stay independent.
*Alternative:* one fixed, well-known endpoint with a broker process that all hosts register with. Rejected: needs a daemon lifecycle and fails when two users share a machine.
Atomic write: write to `<pid>.json.tmp`, then rename. On Windows the file gets an ACL restricted to the current user (inherited from `%LOCALAPPDATA%`, which is already per-user).

### Stale detection
Registration includes the process start time (Linux: `/proc/<pid>/stat` field 22; Windows: `GetProcessTimes`) and process name. A registration is valid only if a process with that pid, start time and name exists. This handles crashed hosts and pid reuse. Readers delete stale files; writers overwrite their own pid's file.

### Protocol shape
Line-delimited JSON (`serde_json`, one object per line, `type` tag):
- client → endpoint: `hello {version, client, update_interval?}` (a future `tcp` kind adds `token`); `subscribe_traffic {node, enabled}`; `subscribe_updates {node, mode: "coalesced" | "all"}`.
- endpoint → client: `welcome {version, process}`, `snapshot {...}`, `event {kind, ...}`, `counters {node, rx, tx, per_second}`, `lagged {dropped}`, `error {message}`.
Version is `major.minor`; minor additions are optional fields that old readers ignore. The structure report is sent as embedded JSON, values in their SECoP JSON encoding, so the TUI can reuse `must-types` to interpret them.

### Events `must-server` adds
`NodeEvent` (non-exhaustive) gains `ClientActivated { id, module: Option<String> }` and `ClientDeactivated { id, module: Option<String> }` (`None` for the whole node). Parameter values in update events (`Updated { line }`) and per-node log records (`Log { level, module, message }`) come from `add-node-log-files`, which this change builds on.

### Nodes that do not run (C ABI)
A C ABI node exists from `SECoP_S_createNode` on, long before (or without) running, and its build problems are what users most need to see. `must-monitor` therefore keeps a registry of node entries independent of `RunningNode`: `must-ffi` registers each node at `createNode` with its context ID (state `building`), reports `nodeComplete` results (`failed` with the problem and warning messages, or `running` with the `RunningNode` whose events it then follows), and removes it at `deleteNode`/`doneLibrary`. Rust nodes register when they start. The endpoint exists while the registry is non-empty.

### Where the code lives
`must-monitor` holds what endpoint and readers share and does not depend on `must-server`: protocol types, the registry, the platform code (process identity, user SID; the only `unsafe` code, Windows API calls) and the transport seam, plus the client library. The endpoint itself is `must_server::monitor`, because snapshots need the node's internals (value cache, structure report) that `must-server` does not expose. This keeps the dependency one-way (`must-server` → `must-monitor`; `must-tui` → `must-monitor`).
The endpoint runs on its own thread with a current-thread tokio runtime, started with the first node and ended (thread joined) with the last, so a host such as LabVIEW has no must monitoring thread left once its nodes are gone, independent of which runtime the nodes use.

### Event pipeline
A per-process `Monitor` singleton (created lazily when the first node is registered) subscribes to every node's `NodeEvent` channel. It fans events out to per-monitor bounded queues (capacity 4096); full queues drop oldest and count. Traffic texts are copied only when at least one monitor subscribed for that node, so unobserved nodes pay only a counter increment. Values come from the module caches for the snapshot, so a connecting monitor never triggers hardware reads.

### Coalesced parameter updates
Polled or pushed parameters can update far faster than a display needs, so per monitor the endpoint keeps a "dirty" map (node, module, parameter) → latest value and flushes it every interval (default 1 s, chosen in `hello`). A value updated several times within an interval is sent once, with its latest value and timestamp; nothing is lost at the end of a burst because the map holds the last value until the next flush. `subscribe_updates {node, mode: "all"}` switches one node of one monitor to an event per update (for debugging), still subject to the bounded queue and `lagged`. The map is bounded by the number of parameters, so coalescing also caps memory per monitor.
*Alternative:* opt-in updates only (snapshot values otherwise frozen). Rejected: a status display wants current values without the cost of every update.

### Tracing integration
What nodes log about themselves arrives as `NodeEvent::Log` with node and module, whatever the host's `tracing` setup. Other records of the process come from `must_server::monitor::log_layer()`, a `tracing_subscriber::Layer` (info and above) that Rust hosts add to their own subscriber; the C ABI calls `install_log_capture()` at `SECoP_S_initLibrary`, which installs a subscriber with only this layer unless the process already has one. `must-server` does not install a global subscriber on its own when a node starts: a Rust host that sets up `tracing` later (e.g. `tracing_subscriber::fmt().init()`) would otherwise panic. The layer skips `must_server::node` records (they come as events) and hands records to a broadcast channel, never blocking or taking the endpoint's locks.

## Risks / Trade-offs

- [Another local process of the same user can read the feed] → Same trust boundary as the user's own files; the feed is read-only. Documented.
- [Windows named pipes are readable by Everyone with the default security descriptor] → Explicit descriptor for the user's SID; a test checks that the pipe's DACL contains no other user or group than the user and SYSTEM.
- [Pipe-name squatting by another process] → `FILE_FLAG_FIRST_PIPE_INSTANCE` on the endpoint side; the client checks the server's pid against the registration.
- [Unix socket paths are limited to about 108 bytes] → `<registry dir>/<pid>.sock` stays short for `$XDG_RUNTIME_DIR` and `/tmp/must-<uid>`; if the path is too long, monitoring is disabled with a warning.
- [Debugging without `nc`] → Documented commands: `socat - UNIX-CONNECT:<path>` on Linux, a PowerShell `NamedPipeClientStream` snippet on Windows.
- [Two platform-specific listeners to maintain] → Kept small behind the seam and tested on all three CI targets.
- [Global `tracing` subscriber conflicts with a Rust host's own] → Never replace an existing global subscriber; document the layer for such hosts.
- [Large structure reports in snapshots] → Sent once per monitor connection; acceptable.

## Migration Plan

Not applicable: new functionality, on by default, off with `MUST_MONITOR=0`.
