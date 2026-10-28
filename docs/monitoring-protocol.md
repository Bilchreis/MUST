# Monitoring

Every process that runs must SEC nodes (a Rust program using `must-server`, or a C or
LabVIEW program using the SHALL C ABI) offers one local **monitoring endpoint**. Through
it, tools such as `must-tui` see the process's nodes, their clients, the messages on the
wire, parameter values and log records. The endpoint is read-only and never slows down a
node.

Monitoring is on by default. Turn it off with the environment variable
`MUST_MONITOR=0`, or in Rust with `must_server::monitor::set_enabled(false)` before the
first node starts.

## Finding endpoints

Each process registers its endpoint as `<pid>.json` in a per-user directory:

| Platform | Registry directory |
|---|---|
| Linux | `$XDG_RUNTIME_DIR/must`, or `/tmp/must-<uid>` without `XDG_RUNTIME_DIR` (mode 0700; a directory owned by someone else or accessible by others is refused) |
| Windows | `%LOCALAPPDATA%\must\run` |

`MUST_MONITOR_DIR` overrides the directory (for tests and special setups); readers must
then use the same value.

A registration looks like this:

```json
{
  "version": "1.1",
  "pid": 4242,
  "name": "LabVIEW.exe",
  "start": 133748224000000000,
  "start_time": 1793118000.5,
  "endpoint": { "kind": "pipe", "name": "\\\\.\\pipe\\must-S-1-5-21-...-1001-4242" }
}
```

`endpoint` describes how to connect, by `kind`:

| `kind` | Fields | Platform |
|---|---|---|
| `unix` | `path`: the Unix domain socket, `<registry dir>/<pid>.sock` (mode 0600) | Linux |
| `pipe` | `name`: the named pipe `\\.\pipe\must-<user SID>-<pid>`, accessible only to the user and SYSTEM | Windows |

Readers skip kinds they do not know (later versions may add e.g. a local TCP kind) and
leave those files alone. `start` (the process start time in the operating system's units)
and `name` let readers tell a live registration from one left behind by a crashed
process whose pid was reused; such stale files, and their socket files, are deleted by
readers. On Windows a reader also checks that the pipe is served by the registered pid.

Only processes of the same user can connect: the operating system enforces it through the
socket's and directory's permissions on Linux and the pipe's security descriptor on
Windows. Nothing listens on a network port.

## Protocol

UTF-8 JSON objects, one per line, each with a `type` field. The protocol version is
`major.minor` (currently `1.1`). Minor versions only add optional fields, message types
and event kinds; readers ignore fields, types and kinds they do not know (the
`must-monitor` client library skips them).

| Version | Additions |
|---|---|
| 1.0 | first version |
| 1.1 | `log_file` in node entries, `log_file` event |

### Opening

The monitor's first line must be a hello; otherwise the endpoint closes the connection
without a word:

```json
{"type":"hello","version":"1.1","client":"my-tool","update_interval":1.0}
```

`update_interval` (optional, 0.1 to 60 s, default 1 s) sets how often coalesced
parameter updates are sent. A hello with another major version gets
`{"type":"error","message":"..."}`, then the connection closes.

The endpoint answers with `welcome` and a `snapshot`:

```json
{"type":"welcome","version":"1.1","process":{"pid":4242,"name":"LabVIEW.exe","library_version":"0.1.0","start_time":1793118000.5}}
{"type":"snapshot","process":{...},"nodes":[...],"logs":[...]}
```

Each node in the snapshot:

| Field | Meaning |
|---|---|
| `id` | identifies the node within the process; used by events and subscriptions |
| `equipment_id` | the node's name |
| `address` | SECoP listen address, once running |
| `state` | `building` (C ABI, not complete yet), `failed`, `running` or `stopped` |
| `context` | SHALL context ID for C ABI nodes, else absent/null |
| `messages` | errors (when failed) and warnings of building the node |
| `structure` | the `describe` data, once running |
| `parameters` | `{module, parameter, report, error}` for every cached parameter: `report` is the SECoP data report `[value, {"t": ...}]`, or the error report if `error` is true; values come from the cache, the hardware is never read for a snapshot |
| `clients` | `{id, peer, connected_at, active_all, active_modules}` per SECoP client |
| `log_file` | the path of the log file the node writes now (see [log-files.md](log-files.md)); absent while file logging is off, not yet started or stopped after an error (1.1) |

`logs` holds up to the 2000 most recent log records `{time, level, node, module, message}`.

### Events and other messages

After the snapshot the endpoint sends:

- `{"type":"event","kind":...}` with `kind`:
  `node_created`, `node_started` (address, structure, warnings), `node_stopped`,
  `node_failed` (messages), `node_removed`,
  `client_connected`, `client_disconnected` (reason), `client_activated`,
  `client_deactivated` (module, or none for the whole node),
  `message` (only for nodes with traffic subscribed: `client`, `direction`
  `received`/`sent`, `time`, `text` cut to 4 KiB with `truncated`),
  `update` (a parameter's `report`/`error`, see below),
  `log` (a log record),
  `log_file` (`node`, and `path` of the file the node writes now: after the first line,
  at midnight, or when it falls back to `<date>-<pid>.log`; without `path` when file
  logging stopped after an error; 1.1).
- `{"type":"counters","node":1,"rx":120,"tx":340,"rx_per_second":2.0,"tx_per_second":5.0}`
  for every running node once per second.
- `{"type":"lagged","dropped":17}` when the monitor did not read fast enough and that many
  events were dropped (each monitor has a queue of 4096 messages).

Parameter updates are **coalesced** by default: at most one `update` per parameter per
interval, with the latest value; the last value of a burst is never lost.

### Requests

A monitor may send, at any time after the hello:

```json
{"type":"subscribe_traffic","node":1,"enabled":true}
{"type":"subscribe_updates","node":1,"mode":"all"}
```

`subscribe_traffic` turns `message` events of a node on or off (off by default).
`subscribe_updates` with `"all"` sends one `update` per value change of that node (for
debugging), `"coalesced"` returns to the default. Nothing else can be changed through the
endpoint.

## Trying it by hand

Linux, with OpenBSD netcat or socat:

```bash
REG=${XDG_RUNTIME_DIR:+$XDG_RUNTIME_DIR/must}; REG=${REG:-/tmp/must-$(id -u)}
ls $REG                                          # 4242.json 4242.sock ...
printf '{"type":"hello","version":"1.1","client":"me"}\n' | nc -U -q 5 $REG/4242.sock
# or: socat - UNIX-CONNECT:$REG/4242.sock
```

Windows (PowerShell):

```powershell
Get-ChildItem $env:LOCALAPPDATA\must\run
$reg  = Get-Content $env:LOCALAPPDATA\must\run\4242.json | ConvertFrom-Json
$name = $reg.endpoint.name -replace '^\\\\\.\\pipe\\', ''
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $name, 'InOut')
$pipe.Connect(5000)
$w = New-Object System.IO.StreamWriter($pipe); $w.AutoFlush = $true
$r = New-Object System.IO.StreamReader($pipe)
$w.WriteLine('{"type":"hello","version":"1.1","client":"me"}')
$r.ReadLine(); $r.ReadLine()    # welcome and snapshot
```

## Log records

What a node logs about itself (handler errors, invalid values, closed slow clients) is
always captured with its node and module. Other `tracing` records of the process (info
and more severe) are captured by `must_server::monitor::log_layer()`: Rust hosts add it to
their subscriber,

```rust
use tracing_subscriber::prelude::*;
tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer())
    .with(must_server::monitor::log_layer())
    .init();
```

and hosts without a subscriber call `must_server::monitor::install_log_capture()`, as the
SHALL C ABI does in `SECoP_S_initLibrary`. must never replaces a subscriber the host set.

## Reading endpoints from Rust

The `must-monitor` crate's client library lists and reads endpoints:

```rust
use must_monitor::client::{self, Options};
use must_monitor::registry::EndpointStatus;

for listed in client::list_endpoints()? {
    if listed.status == EndpointStatus::Live {
        let mut connection = client::connect(&listed.registration, &Options::new("my-tool")).await?;
        println!("{:?}", connection.snapshot().nodes);
        while let Some(message) = connection.next().await? {
            println!("{message:?}");
        }
    }
}
```
