# Proposal

## Why

In SHALL's polling mode every request goes into the queue as soon as it arrives, so a host can fetch and answer several at once, and reads of one parameter share one answer. must calls a module's handler for one request at a time, also in polling mode. The next request therefore reaches `SECoP_S_getStoredCommand` only after the previous one was answered, and duplicates wait unseen in must-server's module queue. With a host loop that calls GET CMD until it returns no data, 4 waiting reads take 200 ms instead of 50 ms (50 ms loop), 8 reads of one parameter take 400 ms, and a slow-tier poll tick stalls client requests for up to 728 ms. A request the host never answers blocks its module for the 60 s timeout. Two host loops that serve different parameters of one module, as SHALL-LabVIEW's filter VIs intend, wait on each other.

The queueing happens in must-server, so the fix belongs there. It also helps Rust handlers whose reads wait on slow I/O.

## What Changes

- **Concurrent requests (opt-in per module).** A module can declare that its handler accepts overlapping calls. Its requests (`read`, `change`, `do`, poll ticks) then start as they arrive instead of one after another, up to a limit per module. Modules that do not opt in keep one call at a time. Requests of one connection stay in order, as today.
- **Shared reads.**
  - On every module, a `read` that waits behind other requests joins a waiting read of the same parameter, unless a `change` or `do` of the module is waiting between them. Both clients get the same reply from one handler call.
  - On a module with concurrent requests, a `read` of a parameter that is already being read joins that read, as in SHALL.
  - Polls and client reads of the same parameter share reads the same way.
- **Polls started together.** On a module with concurrent requests, the default poll hook starts the reads of all due parameters at once.
- **C ABI:** modules in polling mode use concurrent requests. Callback mode keeps one call per module at a time, as in SHALL and as `COMPATIBILITY.md` promises. A shared read is one stored request; its answer replies to every waiting client.
- **`COMPATIBILITY.md`:** polling-mode requests of one module are handed out as they arrive and reads of one parameter are merged, as in SHALL.

No breaking change: the opt-in defaults to off, and merging waiting reads only saves handler calls.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `sec-node-runtime`: "Request concurrency" gains the concurrent-requests opt-in; new requirement "Shared reads".
- `shall-server-abi`: "Polling mode" hands out a module's requests as they arrive and merges reads of one parameter; "Callback mode" states one call per module at a time.

## Impact

- `crates/must-server`: module actor (`runtime/actor.rs`), module builder (`model.rs`), handler context and default poll hook (`handler.rs`), tests.
- `crates/must-ffi`: switching polling-mode modules to concurrent requests (`server/build.rs`); the polled-request path (`server/handler.rs`, `server/state.rs`) must tolerate several requests per module and one waiter giving up while others wait.
- `crates/must-ffi-ctests`: C tests for parallel and merged polling-mode requests.
- Docs: `COMPATIBILITY.md`.
- Depends on `sample-node-polling` (poll hook, tiers), which is implemented but not yet archived; this change does not modify its requirements, so the two can be archived in either order.
