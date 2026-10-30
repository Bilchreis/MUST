# Design

## Context

See proposal.md for the motivation.

Today (`crates/must-server/src/runtime/actor.rs`):
- Every module has one actor task that receives `Job`s (`Read`, `Change`, `Do`, `Refresh`, `Poll`) from a bounded channel of 64 and runs them strictly one after another. Connections send a job and await its reply; poll ticks use `try_send` and are skipped while the channel is full.
- Each handler call runs through `guarded`, which applies the node timeout and turns a panic into `InternalError`.
- A job publishes its side effects before it replies (requirement "Ordering of side effects"); a `change` publishes the changed parameter to all activated clients before `changed`, as SECoP requires.
- The default poll hook (`ModuleHandler::poll`, from `sample-node-polling`) calls `self.read` for each due parameter in turn and publishes through `Ctx`.

In `must-ffi` polling mode (`server/handler.rs`), `ShallModule` turns each handler call into a request in the context's queue (`server/state.rs`) and awaits `SECoP_S_putCommandAnswer`. The queue, the handed-out map and the `Queued` guard already handle any number of requests per module. What limits them to one per module is the actor above.

Measured on the current code, with a host loop that calls GET CMD until it returns no data, every 50 ms: 4 waiting reads take 200 ms, 8 reads of one parameter take 400 ms, a slow-tier tick of 11 parameters stalls client requests by up to 728 ms.

SECoP's `read` asks for "a reasonable recent 'current' value"; a cached or the most recently obtained value is acceptable. After a `change` has made a module BUSY, "all read requests will also reveal a BUSY status-code". The rules for sharing reads follow from these two statements.

## Goals / Non-Goals

**Goals:**
- Polling-mode C modules behave like SHALL's: every request is visible to the host as it arrives, reads of one parameter are one request.
- One mechanism in `must-server`, usable by Rust handlers too.
- No change for modules that do not opt in, apart from sharing reads, which only saves handler calls.
- A client's `read` sent after a `changed` (or `done`) reply never gets a value read before that change.

**Non-Goals:**
- Concurrent requests within one connection. A connection still waits for each reply before reading its next request (requirement "Request concurrency").
- Concurrency in callback mode. SHALL calls one module's functions on one thread.
- `SECoP_S_filterStoredCommand[2]` and the other functions of issue #14.
- Sharing `change` or `do` requests.

## Decisions

### Opt-in on the module builder
`Module::concurrent_requests()` marks a module; `ModuleModel` keeps a `concurrent: bool`. The limit is fixed at 64 running handler calls per module, the size of today's job channel, so a module that opts in cannot hold more work than a module that does not.

*Alternative:* a `ModuleHandler::concurrent()` trait method. Rejected because polling configuration and similar per-module settings already live on the builder, and `must-ffi` decides it per module while building, not per handler type.

### Shared module state, and an actor that always receives
Each module gets one `Mutex`-protected state, shared by its actor and by `Ctx`:
- the list of waiting jobs,
- the table of running reads: parameter → (start epoch, reply senders),
- the module's epoch, a counter.

The actor runs every job as a task and keeps receiving from its channel while jobs run, appending to the waiting list, so the list is always current. In one-call-at-a-time mode it starts the next waiting job only when the previous one has finished. With concurrent requests it starts each waiting job as soon as a permit of a `Semaphore(64)` is free. The waiting list plus running jobs are capped at 64 per module; beyond that the actor stops receiving, so connections wait on the full channel and poll ticks are skipped, as today.

*Alternative:* keep awaiting each job inside the actor loop. Rejected: the waiting list would then miss requests that arrive while a job runs, and a poll could not see waiting client reads.

### Epoch: what "after a change" means
A `change` or `do` job increments the module's epoch after its handler returned and its side effects were published, and before it sends its reply. A connection records the epoch when it sends a `Read` job (`Job::Read { since, .. }`). A read records the epoch when it starts. A client's `read` may join a running read only if `start >= since`, i.e. the running read started after every `change`/`do` that had finished when the client sent its request. Hence a client that received `changed` and then reads never joins a read that began before the change (scenario "Read after changed sees the change").

*Alternative considered first:* "a read may not join across a waiting `change` or `do`". Rejected: stricter than SECoP's read semantics for waiting reads, and it did not stop the BUSY race for running reads on modules with concurrent requests.

### Starting a read takes the waiting reads of its parameter
Whenever a read of `p` starts (a client `Read`, a startup `Refresh`, or a poll through `Ctx::read`), it removes from the waiting list every `Read` and `Refresh` of `p` and removes `p` from every waiting `Poll` job (a `Poll` job left empty is dropped). Their reply senders are added to the read. Each of them arrived before the read started, so its value is at least as recent as the request ("Shared waiting reads"). This needs no epoch check.

### Running reads
On a module with concurrent requests, a request to read `p` while `p` is in the running-reads table joins it: a poll always, a client `read` when `start >= since`. Otherwise it starts a new read, which replaces the table entry for later joiners.

A shared read runs as its own task: one guarded handler read, one publish (value or `error_update`), then it removes its table entry and sends the result to every sender. A waiter that goes away only drops its receiver, so the read continues for the others ("First reader leaves"). The read itself ends at the node timeout.

*Alternative:* `futures::future::Shared`. Rejected to avoid adding `futures` for one use; the sender list is a few lines.

### Polls read through `Ctx`
`Ctx` gets `pub async fn read(&self, parameter: &str) -> Result<Reading, SecopError>` that reads like a poll: it joins a running read of the parameter if there is one (always, being a poll), otherwise starts a read that also takes the waiting reads of that parameter, and it publishes the result. `Ctx` carries the module's handler (`Arc<dyn ModuleHandler>`) and the shared module state for this.

The default `ModuleHandler::poll` changes to use `ctx.read`:
- on a module with concurrent requests it starts the reads of all due parameters as tasks of a `JoinSet` and waits for all of them ("Concurrent polls");
- otherwise it reads them one after another, as today.

A handler that overrides `poll` keeps full control and can call `ctx.read` itself.

A poll tick on a module with concurrent requests runs as its own task like any other job, so a tick does not wait for an earlier tick's hanging read; the next tick joins that read instead of calling the handler again ("Hanging read").

### C ABI: polling mode opts in
`must-ffi` calls `Module::concurrent_requests()` in `DraftNode::build_module` when the node's context uses polling mode, and never in callback mode. Nothing else in `must-ffi` shares reads: a shared read reaches `ShallModule::read` once, becomes one stored request, and its single answer goes to every waiter.

From must-server's view a polling-mode read starts when it is put in the stored queue, not when the host fetches it. So a client's `read` sent after a `changed` gets a request of its own if the earlier request for that parameter was queued before the change was answered (scenario "Read after changed" in `shall-server-abi`).

`ShallModule::request` holds the `Queued` guard for as long as the shared read task runs, so a request is removed from the queue only when the shared read ends or times out, never because one of several waiting clients left.

## Risks / Trade-offs

- [A Rust handler opts in without being safe for overlapping calls] → handlers are `Send + Sync` and take `&self` already; the builder documentation states that calls may overlap and that ordering between clients is not guaranteed.
- [A shared read answers a client with a value obtained before a `change` that was still waiting when the client sent its request] → allowed by SECoP's read semantics ("reasonable recent"); a client that waits for `changed` before reading always gets a value from after the change.
- [A poll that shares a read from before a `change` does not re-read the parameter after it] → the change publishes the changed parameter itself, and SECoP makes the handler responsible for publishing other affected parameters (e.g. BUSY status); the next tick polls again.
- [Unanswered polling-mode requests occupy permits] → each frees its permit at the 60 s timeout; shared reads and joined polls add no further requests, so a host that never answers reads holds at most one request per parameter and epoch plus its pending changes.
- [Changes and reads of one parameter from different clients complete in any order on concurrent modules] → as in SHALL; each connection still sees its own requests in order.
- [Timing-based tests are flaky] → tests count handler calls and gate handlers with barriers or channels instead of sleeping where possible.

## Migration Plan

No migration. The opt-in is off for Rust modules unless they call it. C hosts in polling mode get SHALL's behavior back. Rollback is removing the `concurrent_requests()` call in `must-ffi`.
