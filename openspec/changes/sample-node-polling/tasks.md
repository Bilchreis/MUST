# Tasks

## 1. Model

- [x] 1.1 Add `Module::poll_all` and `Module::slow_poll_interval`, change `Module::poll_interval` to tiered, and resolve each parameter's `PollSource` (`Own`/`Fast`/`Slow`/`None`) in `finish_module` from the base interface class; verify with model tests: Drivable/Readable/Writable tiers, `interface_classes` `[]` puts everything in the slow tier, the tiered-and-uniform-together error, and the cached-parameter-interval warning
- [x] 1.2 Recognize a qualifying `pollinterval` parameter (`double`, no negative `min`), warn for a non-qualifying one, exclude it from polling and startup reads, and seed its initial value (own initial value, else the module interval, else 0.0); verify the valid- and integer-pollinterval scenarios and the structure report

## 2. Runtime

- [x] 2.1 Add `ModuleHandler::poll(&self, ctx, due: &[&str])` with the default read-and-publish body, and change `Job::Poll` to carry a parameter list and call the hook under `guarded` (timeout and panic logged once per module); verify the default-hook, one-hardware-transaction and hook-times-out scenarios
- [x] 2.2 Replace the per-parameter poll tasks in `start_polling` with one task per module (due times per parameter, grouping, `try_send` skip, delay-style catch-up, stop token); verify the periodic-read, fast-and-slow-tier (paused time, 130 s), parameter-interval-wins, uniform-interval and parameters-due-together scenarios
- [x] 2.3 Re-time the fast/uniform interval through a per-module `watch` channel fed by `Hub::publish_value` for the `pollinterval` parameter (clamped to 3600 s, 0 = stop); verify the client-tunes-the-rate, handler-adjusts, handler-refuses, zero-stops-polling, application-push-re-times and never-polled scenarios
- [x] 2.4 Add a runtime test for the unchanged-value scenario (two identical poll results → two updates with different timestamps)

## 3. C ABI

- [x] 3.1 Switch `must-ffi/src/server/build.rs` from `module.poll_interval` to `module.poll_all`; update the build test at `pollinterval` 2.0; verify the uniform-polling-kept and parameter-property-wins scenarios with the C tests (`must-ffi-ctests`) in callback mode on Linux, and in CI for Windows x86_64 and i686
- [x] 3.2 Verify the declared-pollinterval-parameter scenario through the C ABI (setter callback called first, then re-timed reads)

## 4. Example node

- [x] 4.1 Add `fastrand` to `[workspace.dependencies]` and as a dev-dependency of `must-server`; add ±5 mK noise to the reported `value` only, keeping `status`/`busy()` on the true temperature
- [x] 4.2 Configure `cryo` with `.poll_interval(1 s)` and a writable, cached `pollinterval` parameter (double, 0–3600 s, initial 1.0) accepted by the handler; remove the `value` push from `simulate`, keeping the `status` push on transitions; update the doc comment with `change cryo:pollinterval 0.2`
- [x] 4.3 Run the example and check by hand with `nc`: after `activate`, `value` updates arrive about once per second with varying last digits while idle; after `change cryo:pollinterval 0.2` they arrive about five times per second; `target`/`ramp` refresh about once a minute

## 5. Callers, docs and checks

- [x] 5.1 Update `tests/runtime.rs` and `tests/log_files.rs` for the new `poll_interval` meaning (`poll_all` where uniform polling is meant)
- [x] 5.2 Document polling (tiers, precedence, `pollinterval`, poll hook, updates for unchanged values) in the `must-server` crate docs/README, note the `Module::poll_interval` change, and add the `pollinterval`-parameter entry to the "Library behavior" section of `COMPATIBILITY.md`
- [x] 5.3 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, and cross-check `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc` builds as CI does

## 6. Default polling and the C ABI rules

- [ ] 6.1 Model: replace the module's tiered/uniform/slow fields with one `PollMode` (`Tiered { fast }` defaulting to 5 s, `Uniform`, `Off`) plus the slow interval; add `Module::no_polling()`; reject more than one of `poll_interval`/`poll_all`/`no_polling`; warn for `slow_poll_interval` without tiers and for a `pollinterval` parameter on a module with polling off; seed `pollinterval` from the default 5 s; update the model tests (default tiers, no-base-class slow tier without configuration, polling-off-with-uniform error, `pollinterval` alone uses the default tiers)
- [ ] 6.2 Runtime: verify the default-tiers (paused, 130 s: 26/26/2/2), module-without-a-fast-tier and polling-turned-off scenarios in `tests/polling.rs`; switch the polling tests that expect no polling to `no_polling()`
- [ ] 6.3 Existing tests: run the workspace tests and give `no_polling()` to the test modules whose call counts, quiet clients or log contents default polling disturbs (`tests/runtime.rs`, `tests/log_files.rs`, `tests/monitor.rs`, `tests/slow_clients.rs`, `tests/events.rs`, `must-tui`, `must-ffi` tests); record which ones needed it
- [ ] 6.4 C ABI: in `must-ffi/src/server/build.rs` choose per module between a qualifying `pollinterval` parameter (no poll calls, ignored properties reported as warnings for `SECoP_S_showErrors`), SHALL's properties (`poll_all`, or `no_polling` with only parameter properties; module property 0 → `no_polling`) and the defaults; verify all six `shall-server-abi` scenarios with C tests in `must-ffi-ctests` (callback mode, plus polling mode receiving stored `read` requests), and check existing C tests that default polling disturbs
- [ ] 6.5 Docs: update the crate docs and README (polling by default, `no_polling`), and `COMPATIBILITY.md` (the three C cases; modules without configuration are now polled, also in polling mode; module property `pollinterval` 0 restores "nothing polled"; ignored properties next to a `pollinterval` parameter)
- [ ] 6.6 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, and `cargo check` for `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc`

## Workflow follow-up

- Archive after review.
