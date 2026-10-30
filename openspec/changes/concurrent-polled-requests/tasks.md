# Tasks

## 1. Module option

- [ ] 1.1 Add `Module::concurrent_requests()` and `ModuleModel::concurrent` in `crates/must-server/src/model.rs`, documented with the overlap and ordering caveats; verify with a model test that the flag is off by default and set by the builder

## 2. Actor and shared reads

- [ ] 2.1 Give each module a shared state (waiting jobs, running reads with start epoch and reply senders, epoch counter) used by the actor and `Ctx`; let the actor run each job as a task and keep receiving into the waiting list while jobs run, starting the next job only after the previous one finished, with at most 64 waiting and running jobs; verify with the existing runtime and polling tests and the "One call at a time by default" scenario
- [ ] 2.2 When a read of a parameter starts, take every waiting `Read`/`Refresh` of it and remove it from waiting `Poll` jobs (dropping empty ones); one handler read, one publish, replies to all; verify the "Reads waiting behind a change", "Reads around a waiting change", "Poll tick across a change" and "Shared error" scenarios with runtime tests that count handler calls
- [ ] 2.3 Add the epoch: `change`/`do` jobs increment it after publishing and before replying, `Job::Read` carries the epoch at sending, a read records it at start; verify with a unit test of the join rule (`start >= since`)
- [ ] 2.4 Run jobs of modules with concurrent requests as soon as a permit of a `Semaphore(64)` is free; a read of a parameter that is running joins it (polls always, client reads when `start >= since`); the shared read runs as its own task and keeps running when a waiter goes away; verify the "Concurrent requests", "Order within a connection", "Client joins a client's read", "Read after changed sees the change" and "First reader leaves" scenarios, and that a 65th running request waits until one finishes
- [ ] 2.5 Check that side effects still precede replies on modules with concurrent requests; verify the "Status update before changed" case of the "Ordering of side effects" requirement on such a module

## 3. Polls

- [ ] 3.1 Add `Ctx::read` (reads like a poll: joins a running read of the parameter, else starts one that takes the waiting reads of that parameter, and publishes the result) and give `Ctx` the module's handler and shared state; document it on `Ctx`
- [ ] 3.2 Change the default `ModuleHandler::poll` to read through `Ctx::read`, all due parameters at once on modules with concurrent requests, one after another otherwise; run poll ticks of such modules as their own tasks; verify the "Reads started together", "Hanging read", "Client joins a poll" and "Poll joins a client's read" scenarios, and that the existing polling tests (`tests/polling.rs`) still pass
- [ ] 3.3 Document concurrent requests, shared reads (with the rule for reads after a `change` or `do`) and the poll hook's behavior in the `must-server` crate docs

## 4. C ABI

- [ ] 4.1 Call `Module::concurrent_requests()` for modules of polling-mode contexts in `crates/must-ffi/src/server/build.rs`, never in callback mode; verify with a unit test of the build
- [ ] 4.2 Add C tests in `crates/must-ffi-ctests` (with their Rust drivers in `tests/server.rs`) for the polling-mode scenarios "Requests of one module at once", "Answers in any order", "Reads of one parameter merged", "Read after changed", "Poll of a queued parameter", "Unanswered read does not block the module" and "Poll tick queued at once"
- [ ] 4.3 Add a callback-mode C test for "One module at a time" (a blocking getter of `mod1:a` delays the getter of `mod1:b` but not a read of another module)
- [ ] 4.4 Update `COMPATIBILITY.md` ("Callbacks" and "Polled requests" under "Library behavior"): polling-mode requests are handed out as they arrive, reads of one parameter are merged except for a read sent after a `changed` or `done` reply, answers may come in any order

## 5. Checks

- [ ] 5.1 Re-run the latency benchmark from the audit (C host polling every 50 ms, calling GET CMD until it returns no data) and confirm: 4 waiting reads ≈ 50 ms, 8 reads of one parameter ≈ 50 ms, no client stall from a slow-tier tick
- [ ] 5.2 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686

## Workflow follow-up

- Archive `sample-node-polling` before or after this change; this change adds requirements next to its poll hook but does not modify them.
- Archive this change after review.
