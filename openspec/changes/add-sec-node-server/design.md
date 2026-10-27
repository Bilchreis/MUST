# Design

## Context

Builds on `must-types` (change `add-secop-data-types`: datainfo, values, identifiers, postfix type rules). SHALL's server (`SHALL-ServerLib`) is Qt-based: one `QThread` per node, module and connection (`SECoP_S_setManyThreads`), getter/setter callbacks or a polling queue, a status GUI, and SECoP V2019-09-16 on the wire. must keeps the observable behavior that SHALL users rely on (nodes built in code, callbacks or queued requests, periodic polling, pushed updates) but speaks SECoP 2.0. The C ABI in `add-shall-c-abi` will drive this crate, so its API has to fit both idiomatic Rust callers and the SHALL call pattern (build by "focus", answer requests later from another thread).

## Goals / Non-Goals

**Goals:**
- A Rust API that makes a correct node easy and an incorrect one impossible to start.
- One code path for all handler styles: async Rust handlers, synchronous C callbacks and SHALL's polling queue.
- Observability hooks that `add-node-monitoring` can subscribe to without touching protocol code.

**Non-Goals:**
- `check`, `logging`, WebSocket transport, UDP discovery, `matrix`.
- Serial-line transport and `error_closed` (structure changes after start are not possible).
- Access control (SECoP has none; `visibility` is only a hint to clients).

## Decisions

### Crate layout and runtime
`crates/must-server` on `tokio` (multi-threaded runtime). Modules: `model` (declaration and validation), `report` (structure report), `protocol` (framing, parsing, formatting; no I/O), `runtime` (node task, connections, dispatcher, cache, polling). `protocol` is pure and unit-tested on strings; `runtime` is tested end-to-end over loopback TCP.
A node can be started on a caller's runtime (`Node::spawn(handle)`) or on a library-owned one (`Node::start()`, which lazily creates a shared runtime). The C ABI uses the latter, since its callers have no runtime.
*Alternative:* thread per connection like SHALL. Rejected: async tasks scale to many connections and make timeouts and cancellation simple; it also removes the need for `SECoP_S_setManyThreads`.

### Declaration API
`NodeBuilder` → `ModuleBuilder` → `ParameterBuilder` / `CommandBuilder`, all accumulating properties as typed fields plus a `custom` map. `NodeBuilder::build()` runs every check and returns either a `Node` or a `Vec<ModelError>` with paths like `t1:target.visibility`. The C ABI's "focus" style (`SECoP_S_addModule` then `SECoP_S_addProperty...` on the last item) maps onto a cursor over the same builders.

### Handler trait
```rust
#[async_trait]
trait ModuleHandler: Send + Sync {
    async fn read(&self, ctx: &Ctx, param: &str) -> Result<Reading, SecopError>;
    async fn change(&self, ctx: &Ctx, param: &str, value: Value) -> Result<Reading, SecopError>;
    async fn call(&self, ctx: &Ctx, cmd: &str, arg: Option<Value>) -> Result<Reading, SecopError>;
}
```
`Reading { value, t: Option<f64>, e: Option<Value> }`. `Ctx` lets the handler push side-effect updates (`ctx.update("status", ...)`) that are guaranteed to go out before the reply. Default methods return `NotImplemented`-style errors so simple modules implement only `read`. The C ABI implements this trait once: for callbacks it calls the C function on a blocking thread (`spawn_blocking`); for polling mode it enqueues the request and awaits a oneshot answered by `SECoP_S_putCommandAnswer`.

### Per-module actor and update bus
Each module runs as an actor task owning its cache and a request queue; this gives "one handler call at a time per module" without locks and natural ordering of side effects: the actor publishes updates to the node's update bus before it completes the request, and the connection task writes the reply only after the bus delivered those updates to all activated connections (updates are written in bus order, replies are queued behind them). "Delivered" means enqueued: each connection has its own outgoing queue drained by its own writer task, so the bus never waits for a socket.

### Slow clients
Each outgoing queue tracks the bytes it holds (bytes already handed to the socket no longer count). Two kinds of messages are treated differently against the node's `max_outgoing_bytes` (builder option, default 16 MiB):
- **Replies** to the connection's own requests (`reply`, `changed`, `done`, `describing`, `pong`, `active`, error replies) are always enqueued, whatever their size. Instead, the connection's reader checks the queue before taking the next request from the socket: while the queue holds more than the limit, it waits until the writer has drained it below the limit. TCP flow control then slows the client down. Since requests of one connection are processed in order, a connection can never have more than the limit plus one reply buffered, even if its client floods requests without reading.
- **Unsolicited updates** (`update`, `error_update`) are never delayed, because the update bus serves all connections. If enqueueing one would take the queue beyond the limit, the connection is closed, its queue dropped and a `warn` log written with the client address.

This keeps large legitimate replies (a big `describe`, later a `get_data` matrix of an `AcquisitionChannel`) safe regardless of how many updates happen to be queued, while a client that ignores updates is still dropped. A client that both floods requests and is activated is dropped by the update rule. 16 MiB is far above normal update bursts yet small enough for a 32-bit LabVIEW host.
For comparison: SHALL queues without limit and never disconnects (memory grows in the host); Frappy sends synchronously from the updating thread while holding the module's update lock and drops a client only after a 60 s send timeout, stalling that module's updates for everyone meanwhile.
*Alternative:* additionally disconnect when a queue has not drained for N seconds. Not added now; the byte limit already bounds memory, and a time limit can be added later as another option.

### Message parsing
Split at the first space (action) and the next space (specifier); the rest is data. Data is parsed with `serde_json` into `serde_json::Value`, then decoded with `must-types` in request mode. Replies are formatted from typed values, never by string concatenation of user data. Maximum line length is a node option (default 1 MiB) because `describe` replies of big nodes can be long, while incoming requests are small.

### Draft identification
`*IDN?` replies `ISSE,SECoP,<draft-date>,v2.0` with the commit date of the pinned SECoP spec revision (from `must-types`). Once SECoP 2.0 is released, the field becomes empty (`ISSE,SECoP,,v2.0`).

### Constant parameters
The spec says a constant parameter "can neither be read nor written". must answers a `read` with the constant value (no handler call) and a `change` with `ReadOnly`, and leaves it out of initial updates. Answering reads is harmless and friendlier to clients that read every parameter.

### Errors
`must-types` gains `ErrorClass` with all SECoP 2.0 classes (`ProtocolError`, `NoSuchModule`, `NoSuchParameter`, `NoSuchCommand`, `ReadOnly`, `NotCheckable`, `WrongType`, `RangeError`, `BadJSON`, `NotImplemented`, `HardwareError`, `CommandRunning`, `CommunicationFailed`, `TimeoutError`, `IsBusy`, `IsError`, `Disabled`, `Impossible`, `ReadFailed`, `OutOfRange`, `InternalError`) and `SecopError { class, message, extra }`. A handler panic is caught and turned into `InternalError`.

### Observability
All runtime events (node started/stopped, client connected/disconnected, message in/out, update published) are emitted as `tracing` events with structured fields, and the node exposes a `subscribe()` broadcast channel of `NodeEvent`s. `add-node-monitoring` consumes both; this change only defines and emits them.

## Risks / Trade-offs

- [Spec text and examples disagree on the error report's first element (prose says "the request message", examples and the reports chapter say the error class)] → Follow the examples and the reports chapter: `[class, text, {..}]`.
- [Strict validation may reject node definitions that SHALL accepted with warnings] → Model errors carry a severity; the C ABI change maps "missing optional property" style issues to SHALL's `SECoP_S_WARNING_*` codes instead of failing.
- [A client on a slow but working link might be disconnected during a very large burst of updates] → The limit is configurable per node; the client recovers by reconnecting and activating again.
- [A very large reply (e.g. a future `get_data` matrix) makes updates for that client wait behind it and can exceed the node's reply `timeout`] → Inherent to one TCP connection; when `matrix` is added, encode it streaming into the output buffer and document raising `timeout` for such nodes.
- [Async handler trait needs `async_trait` (boxing) until native async traits in dyn position are stable] → Cost is one allocation per request, negligible compared to I/O.

## Open Questions

- Default bind interface when none is given: SHALL binds all interfaces; keep that unless the C ABI change needs otherwise.
