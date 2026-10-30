# Design

## Context

See proposal.md for the motivation.

Today (`crates/must-server/src/runtime/actor.rs`):
- Every module has one actor task that receives `Job`s (`Read`, `Change`, `Do`, `Refresh`, `Poll`) from a bounded channel of 64 and runs them strictly one after another. Connections send a job and await its reply; poll ticks use `try_send` and are skipped while the channel is full.
- Each handler call runs through `guarded`, which applies the node timeout and turns a panic into `InternalError`.
- A job publishes its side effects before it replies (requirement "Ordering of side effects").
- The default poll hook (`ModuleHandler::poll`, from `sample-node-polling`) calls `self.read` for each due parameter in turn and publishes through `Ctx`.

In `must-ffi` polling mode (`server/handler.rs`), `ShallModule` turns each handler call into a request in the context's queue (`server/state.rs`) and awaits `SECoP_S_putCommandAnswer`. The queue, the handed-out map and the `Queued` guard already handle any number of requests per module. What limits them to one per module is the actor above.

Measured on the current code, with a host loop that calls GET CMD until it returns no data, every 50 ms: 4 waiting reads take 200 ms, 8 reads of one parameter take 400 ms, a slow-tier tick of 11 parameters stalls client requests by up to 728 ms.

## Goals / Non-Goals

**Goals:**
- Polling-mode C modules behave like SHALL's: every request is visible to the host as it arrives, reads of one parameter are one request.
- One mechanism in `must-server`, usable by Rust handlers too.
- No change for modules that do not opt in, apart from merging waiting reads.

**Non-Goals:**
- Concurrent requests within one connection. A connection still waits for each reply before reading its next request (requirement "Request concurrency").
- Concurrency in callback mode. SHALL calls one module's functions on one thread.
- `SECoP_S_filterStoredCommand[2]` and the other functions of issue #14.
- Merging `change` or `do` requests.

## Decisions

### Opt-in on the module builder
`Module::concurrent_requests()` marks a module; `ModuleModel` keeps a `concurrent: bool`. The limit is fixed at 64 running handler calls per module, the size of today's job channel, so a module that opts in cannot hold more work than a module that does not.

*Alternative:* a `ModuleHandler::concurrent()` trait method. Rejected because polling configuration and similar per-module settings already live on the builder, and `must-ffi` decides it per module while building, not per handler type.

### The actor keeps its own waiting list
The actor drains its channel into a local `VecDeque<Job>` whenever it looks for work, so it can see every waiting job.

- **One call at a time (default):** the actor takes the first waiting job. If it is a `Read` of `p`, it also takes every later waiting `Read` of `p` up to the first waiting `Change` or `Do`, makes one handler call, and sends the result to all of them. This implements "Shared waiting reads". `Refresh` (startup read) jobs join like reads; `Poll` jobs never join, since the hook decides what it reads.
- **Concurrent requests:** the actor starts each job as a task in a `JoinSet` as soon as a permit of a `Semaphore(64)` is free. While no permit is free it stops draining the channel, so connections wait on the full channel and poll ticks are skipped, as today.

*Alternative:* one task per request without an actor. Rejected: the actor keeps the stop and drain logic in one place and makes the one-at-a-time path a special case of the same loop.

### Shared reads: a per-module table of running reads
For a module with concurrent requests, the actor keeps `HashMap<parameter, Vec<oneshot::Sender<JobResult>>>` of running reads.

- A `Read` of `p` with an entry for `p` adds its reply sender and takes no permit.
- Otherwise the actor creates the entry and starts a task that reads through `guarded`, publishes (value or `error_update`) once, removes the entry under the table's lock and sends the result to every sender.
- The task owns the read. A waiter that goes away (connection closed, node timeout of its own job) only drops its receiver, so the read continues for the others ("Shared running reads", scenario "First reader leaves"). The read itself still ends at the node timeout.

The table sits behind a `Mutex` shared by the actor and `Ctx`, so polls can use it (next decision). No new dependency is needed.

*Alternative:* `futures::future::Shared`. Rejected to avoid adding `futures` for one use; the sender list is a few lines.

### Polls read through `Ctx`
`Ctx` gets `pub async fn read(&self, parameter: &str) -> Result<Reading, SecopError>` that reads through the node: on a module with concurrent requests it joins or starts a shared read in the table above, otherwise it calls the handler's `read` directly. It publishes the result like a poll does today. `Ctx` carries the module's handler (`Arc<dyn ModuleHandler>`) and the table for this.

The default `ModuleHandler::poll` changes to use `ctx.read`:
- on a module with concurrent requests it starts the reads of all due parameters as tasks of a `JoinSet` and waits for all of them ("Concurrent polls");
- otherwise it reads them one after another, as today.

Client reads and polls therefore share reads both ways. A handler that overrides `poll` keeps full control and can call `ctx.read` itself.

A poll tick on a module with concurrent requests runs as its own task like any other job, so a tick does not wait for an earlier tick's hanging read; the next tick joins that read instead of calling the handler again ("Hanging read").

### C ABI: polling mode opts in
`must-ffi` calls `Module::concurrent_requests()` in `DraftNode::build_module` when the node's context uses polling mode, and never in callback mode. Nothing else in `must-ffi` merges reads: a shared read reaches `ShallModule::read` once, becomes one stored request, and its single answer goes to every waiter through the table.

`ShallModule::request` holds the `Queued` guard for as long as the shared read task runs, so a request is removed from the queue only when the shared read ends or times out, never because one of several waiting clients left.

## Risks / Trade-offs

- [A Rust handler opts in without being safe for overlapping calls] → handlers are `Send + Sync` and take `&self` already; the builder documentation states that calls may overlap and that ordering between clients is not guaranteed.
- [A joined running read returns a value whose hardware read began before the joining request arrived] → only on modules that opt in; this is SHALL's behavior, and the alternative is a second hardware round trip that SHALL never made.
- [Unanswered polling-mode requests occupy permits] → each frees its permit at the 60 s timeout; merged reads and joined polls add no further requests, so a host that never answers reads holds at most one request per parameter plus its pending changes.
- [Changes and reads of one parameter from different clients complete in any order on concurrent modules] → as in SHALL; each connection still sees its own requests in order.
- [Timing-based tests are flaky] → tests count handler calls and gate handlers with barriers or channels instead of sleeping where possible.

## Migration Plan

No migration. The opt-in is off for Rust modules unless they call it. C hosts in polling mode get SHALL's behavior back. Rollback is removing the `concurrent_requests()` call in `must-ffi`.
