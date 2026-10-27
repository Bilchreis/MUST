# Design

## Context

`must-server` already publishes every node's activity on a per-node `tokio::sync::broadcast` channel of `NodeEvent`s (capacity 1024): node started/stopped, client connected/disconnected (with reason), and every line received or sent (`MessageReceived`/`MessageSent`, with the connection id). A broadcast sender never waits for receivers; a receiver that falls behind gets `RecvError::Lagged(n)`. The node's own log records go only to `tracing` (`info!`/`warn!`/`error!` in `runtime/`), without the node or module as structured context, and only reach a file if the host installed a subscriber. C and LabVIEW hosts reach `must-server` through the SHALL C ABI, whose exported functions cannot change. See proposal.md for motivation and specs/node-log-files for the behavior.

## Goals / Non-Goals

**Goals:**
- A node's file shows exactly what its clients saw, plus why things happened, with no setup.
- No measurable effect on node latency; bounded memory.
- Same behavior on Linux and Windows, 32- and 64-bit.

**Non-Goals:**
- A size limit (retention by days only, as decided for this change).
- Compression, structured (JSON) log files, or log shipping.
- Logging of hosts' own `tracing` records that do not concern a node.
- Changing the in-memory logs of `add-node-monitoring`.

## Decisions

### Feed from `NodeEvent`, plus a new `Log` event
The file logger subscribes to the node's existing broadcast channel, so traffic and connection lines come from the same source the monitor uses, at no extra cost to the node. For the node's own log records, `must-server` adds a `NodeEvent::Log { level, module: Option<String>, message }` (the enum is `#[non_exhaustive]`, so this is not breaking), emitted next to the existing `tracing` calls through one internal helper that does both. This gives node and module attribution without depending on how (or whether) the host set up `tracing`.
*Alternative:* a `tracing` layer that writes files. Rejected: needs a global subscriber that Rust hosts may already own, and node attribution would need spans around every task.
`add-node-monitoring` can use the same `Log` events for its log capture.

### Updates once, from the published value
Every activated client receives its own copy of each `update`, so logging sent lines would repeat each value once per client. Instead `NodeEvent::Updated` gains the published message (`line: Arc<str>`, the `update` or `error_update` text the hub already builds once for all clients), and the logger writes that line once with kind `U`. Sent lines whose action is `update` or `error_update` are skipped; that includes the initial updates after `activate`, whose values were logged when they were published. Values published while no client is activated are logged too, so the file is a complete value history. `add-node-monitoring` gets values in its update events from the same field.
*Alternative:* log only the copy for the first activated client. Rejected: nothing would be logged while no client is activated, and "first" changes as clients come and go.

### Drops happen at the broadcast channel
The logger task never blocks the node, because the broadcast sender never waits. When the task falls behind, `Lagged(n)` tells it exactly how many events it missed; it writes `* dropped <n> lines` before the next line. A second queue between the task and the writer thread (`std::sync::mpsc::sync_channel`, 8192 lines) uses `try_send` and adds its drops to the same counter. The broadcast capacity stays at 1024; the logger task formats lines quickly and should rarely lag.

### One writer thread per process
Formatting happens in a tokio task per node; file I/O (open, write, flush, rename at midnight, deleting old files) happens on one plain thread per process (`must-log-writer`), started with the first logging node. Blocking file I/O never runs on tokio's worker threads, and one thread serves any number of nodes. The thread flushes each open file every second and when a node's stream ends.

### File naming, day change and two processes
The writer keeps one open file per node. Before each line it compares the line's local date with the open file's date and switches files at midnight. A file is opened in append mode and held exclusively for writing while open: on Unix with `std::fs::File::try_lock` (advisory), on Windows with a share mode allowing others only to read and delete it (Windows locks are mandatory and would keep editors from reading the file, and need a handle with more than append access). If the file is held, another process writes that node's file today, and the writer uses `<date>-<pid>.log` instead. The node folder is the `equipment_id` with characters other than ASCII letters, digits, `-`, `_` and `.` replaced by `_` (SECoP identifiers are already safe; `equipment_id` may not be).

### Line format
`<local time, ms, offset> <kind> [<client id> <peer>] <text>`, with kinds `>` (received), `<` (sent), `U` (published update, no client), `*` (connect, disconnect, start, stop, dropped lines) and `INFO`/`WARN`/`ERROR` for log records (with `<module>:` before the text when known). Plain text keeps the files usable with `less`, `grep` and editors; SECoP messages are single lines, so they are written unchanged. Local time with the offset matches what users see on the lab clock while staying unambiguous. Time comes from `chrono::Local` (safe to query from several threads, unlike `time`'s local offset on Unix).

### Retention
The writer deletes, per node folder, files named `YYYY-MM-DD.log` or `YYYY-MM-DD-<pid>.log` whose date is before today minus the retention in days (so with 2 days, on the 27th the files of the 25th and 26th stay and the 24th goes). It does so when a node starts and once per hour, and leaves all other files alone. Only folders of nodes this process runs are cleaned; a node that never runs again keeps its old files (documented).

### Configuration
`LogFiles` settings (`enabled`, `dir`, `days`) are read once per process from `MUST_LOG_FILES`, `MUST_LOG_DIR`, `MUST_LOG_DAYS` and can be overridden per node with `NodeBuilder::log_files(LogFiles)` in Rust. The C ABI has no new functions (its exports must match SHALL's headers), so C and LabVIEW hosts use the environment variables. Defaults: on, 2 days, `$XDG_STATE_HOME/must/logs` or `~/.local/state/must/logs` on Linux (directories created with mode 0700), `%LOCALAPPDATA%\must\logs` on Windows (inherits the per-user ACL).

### Errors
If the directory cannot be created or a file cannot be opened or written (including a full disk), the writer logs one `tracing` warning naming the path, stops logging for that node, and drops its lines from then on. The node keeps working. It retries at the next day change, so a temporary problem does not disable logging for good.

## Risks / Trade-offs

- [No size limit: a node with fast polling can write gigabytes per day] → Documented with an estimate (bytes per update × updates per second; independent of the number of clients, since updates are logged once). Retention can be lowered to 1 day, or logging turned off per process. Write errors from a full disk stop logging instead of affecting the node. A size limit can be added later without changing the format.
- [Log files contain all values exchanged, which may be sensitive measurement data] → Kept in a per-user directory (0700 on Linux, per-user ACL on Windows), documented.
- [Advisory locks are not honored by other tools] → Only must processes write these files; editors only read them.
- [A crash loses up to about 1 s of lines] → Accepted; flushing per line would cost too much at high traffic.
- [Adding `NodeEvent::Log` duplicates each log call] → One internal helper emits both, so call sites stay single.

## Migration Plan

New behavior, on by default. Users who do not want files set `MUST_LOG_FILES=0`. Removing the feature leaves only the files, which are deleted by hand or expire.
