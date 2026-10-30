# Tasks

## 1. Module option

- [ ] 1.1 Add `Module::concurrent_requests()` and `ModuleModel::concurrent` in `crates/must-server/src/model.rs`, documented with the overlap and ordering caveats; verify with a model test that the flag is off by default and set by the builder

## 2. Actor and shared reads

- [ ] 2.1 Give the actor in `runtime/actor.rs` a local waiting list drained from its channel; in one-call-at-a-time mode let a `Read` (or `Refresh`) take every later waiting `Read` of the same parameter up to the first waiting `Change` or `Do`, call the handler once and reply to all; verify the "Reads waiting behind a change", "Change between the reads" and "Shared error" scenarios with runtime tests that count handler calls
- [ ] 2.2 Add the per-module table of running reads (shared by actor and `Ctx`) and the shared-read task: one guarded handler read, one publish, replies to every waiter, entry removed when done; the task keeps running when a waiter goes away; verify the "Client joins a client's read" and "First reader leaves" scenarios
- [ ] 2.3 Run jobs of modules with concurrent requests as `JoinSet` tasks limited by a `Semaphore(64)`; joined reads take no permit; while no permit is free the actor stops draining its channel; verify the "Concurrent requests", "One call at a time by default" and "Order within a connection" scenarios, and that a 65th running request waits until one finishes
- [ ] 2.4 Check that side effects still precede replies on modules with concurrent requests; verify the "Status update before changed" case of the "Ordering of side effects" requirement on such a module

## 3. Polls

- [ ] 3.1 Add `Ctx::read` (reads through the node, joining or starting a shared read on modules with concurrent requests, publishing the result) and give `Ctx` the module's handler and read table; document it on `Ctx`
- [ ] 3.2 Change the default `ModuleHandler::poll` to read through `Ctx::read`, all due parameters at once on modules with concurrent requests, one after another otherwise; run poll ticks of such modules as their own tasks; verify the "Reads started together", "Hanging read", "Client joins a poll" and "Poll joins a client's read" scenarios, and that the existing polling tests (`tests/polling.rs`) still pass
- [ ] 3.3 Document concurrent requests, shared reads and the poll hook's behavior on such modules in the `must-server` crate docs

## 4. C ABI

- [ ] 4.1 Call `Module::concurrent_requests()` for modules of polling-mode contexts in `crates/must-ffi/src/server/build.rs`, never in callback mode; verify with a unit test of the build
- [ ] 4.2 Add C tests in `crates/must-ffi-ctests` (with their Rust drivers in `tests/server.rs`) for the polling-mode scenarios "Requests of one module at once", "Answers in any order", "Reads of one parameter merged", "Unanswered read does not block the module" and "Poll tick queued at once"
- [ ] 4.3 Add a callback-mode C test for "One module at a time" (a blocking getter of `mod1:a` delays the getter of `mod1:b` but not a read of another module)
- [ ] 4.4 Update `COMPATIBILITY.md` ("Callbacks" and "Polled requests" under "Library behavior"): polling-mode requests are handed out as they arrive, reads of one parameter are merged, answers may come in any order

## 5. Checks

- [ ] 5.1 Re-run the latency benchmark from the audit (C host polling every 50 ms, calling GET CMD until it returns no data) and confirm: 4 waiting reads ≈ 50 ms, 8 reads of one parameter ≈ 50 ms, no client stall from a slow-tier tick
- [ ] 5.2 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686

## Workflow follow-up

- Archive `sample-node-polling` before or after this change; this change adds requirements next to its poll hook but does not modify them.
- Archive this change after review.
