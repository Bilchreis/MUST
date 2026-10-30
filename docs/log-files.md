# Log files

Every running must SEC node (a Rust program using `must-server`, or a C or LabVIEW program
using the SHALL C ABI) writes a rolling log of what it exchanged with its clients and what
it reported, so that problems can be investigated afterwards. File logging is on by
default and needs no setup.

## Location

Each node writes one file per day:

```text
<log dir>/<node folder>/<YYYY-MM-DD>.log
```

| Platform | Default log directory |
|---|---|
| Linux | `$XDG_STATE_HOME/must/logs`, or `~/.local/state/must/logs` without `XDG_STATE_HOME` (created with mode 0700) |
| Windows | `%LOCALAPPDATA%\must\logs` |

- The **node folder** is the node's `equipment_id`, with every character other than
  ASCII letters, digits, `-`, `_` and `.` replaced by `_`: `lab:cryo/1` logs into
  `lab_cryo_1`.
- The **date** is the local date at which a line was written; at midnight the node
  continues in the next day's file.
- If another process already writes a node's file of the day (the same `equipment_id` in
  two programs), the second process writes `<YYYY-MM-DD>-<pid>.log` instead, so lines of
  two processes are never mixed.

## Content

One line per event, in UTF-8:

```text
2026-10-27T16:42:10.120+01:00 * node started on 0.0.0.0:2055 (must 0.1.0)
2026-10-27T16:42:10.122+01:00 * [3 192.168.1.20:51234] connected
2026-10-27T16:42:10.123+01:00 > [3 192.168.1.20:51234] change t1:target 300
2026-10-27T16:42:10.125+01:00 U update t1:status [[300,"ramping"],{"t":1793118130.124}]
2026-10-27T16:42:10.126+01:00 < [3 192.168.1.20:51234] changed t1:target [300.0,{"t":1793118130.125}]
2026-10-27T16:42:10.456+01:00 U update t1:value [4.2,{"t":1793118130.456}]
2026-10-27T16:42:11.002+01:00 WARN t1: reading value failed: HardwareError: sensor disconnected
2026-10-27T16:45:00.300+01:00 * [3 192.168.1.20:51234] disconnected: closed by client
2026-10-27T17:00:00.000+01:00 * node stopped
```

Each line starts with the local time (milliseconds, UTC offset), then the kind:

| Kind | Meaning |
|---|---|
| `>` | a message received from a client, with the client's connection number and address |
| `<` | a message sent to a client (complete, including `describing` replies) |
| `U` | a parameter value or error the node published (`update` / `error_update`) |
| `*` | node started or stopped, client connected or disconnected (with the reason), lines dropped |
| `INFO`, `WARN`, `ERROR` | what the node logs about itself, with the module when known (`WARN t1: ...`) |

Updates are logged **once**, when the node publishes them, however many clients are
activated, and also while no client is activated; the copies sent to the individual
clients (including the initial updates after `activate`) are not logged again. Messages are
written unchanged, so the files can be read with `less`, searched with `grep` and opened in
any editor.

## Retention

When a node starts, and then every hour, the files of its folder whose day ended more than
the retention ago are deleted (default 2 days: on the 27th, the files of the 25th and 26th
are kept with today's, the 24th's is deleted). Only files named `YYYY-MM-DD.log` or
`YYYY-MM-DD-<pid>.log` are deleted; other files in the folder are left alone. Folders of
nodes that no longer run are not cleaned; delete them by hand.

There is no size limit. A file grows by about the length of a line (60 to 150 bytes) for
every update and message: a parameter updated once per second adds about 8 MB per day,
independent of the number of clients. With the default retention, a node keeps up to three
days of files (today and two complete days).

## Configuration

C, LabVIEW and Rust programs read these environment variables once, when the first node
starts:

| Variable | Meaning |
|---|---|
| `MUST_LOG_FILES=0` | no log files (no directory is created) |
| `MUST_LOG_DIR` | the log directory instead of the default |
| `MUST_LOG_DAYS` | retention in days, at least 1 (default 2) |

Invalid values are reported as a warning (in the node's log and through `tracing`) and
replaced by the default. For example, on Linux

```bash
MUST_LOG_DIR=/tmp/must-logs MUST_LOG_DAYS=7 cargo run --example simple_node
```

writes `/tmp/must-logs/must-example/<today>.log` and keeps 7 days. On Windows, for a
LabVIEW process, set the variables for the user (System → Environment variables) before
starting LabVIEW, e.g. `MUST_LOG_DIR=D:\logs`.

Rust programs can configure each node; this takes precedence over the environment:

```rust
use must_server::logfiles::LogFiles;

let node = NodeBuilder::new("cryo", "cryostat")
    .log_files(LogFiles::default().dir("/data/must-logs").days(30))
    // or .log_files(LogFiles::off())
    // ...
```

## Behavior

- **Never slows a node.** Lines go through a bounded queue to one background writer thread
  per process. When the disk is too slow and the queue is full, lines are dropped and
  the gap is marked with a line `* dropped <n> lines`. The thread runs while any node of
  the process logs and ends when the last one stops, so a stopped program (e.g. after
  `SECoP_S_doneLibrary`) has no logging thread left.
- **Flushing.** Files are flushed at least once per second and when a node stops, so a log
  is complete up to about one second before a crash of the program, and up to the
  `* node stopped` line after a normal stop.
- **Errors.** If the directory cannot be created or a file cannot be written (read-only
  directory, full disk), the node logs one warning naming the path, stops file logging for
  that day and keeps serving its clients. It tries again with the next day.

## Privacy

The files contain every value the node exchanged with its clients and the clients'
addresses. They are kept in a per-user directory (mode 0700 on Linux, the user's profile
on Windows); a directory set with `MUST_LOG_DIR` is created the same way if it does not
exist, and otherwise keeps its permissions.
