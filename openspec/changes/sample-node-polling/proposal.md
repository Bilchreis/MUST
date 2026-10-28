# Proposal

## Why

The example node (`crates/must-server/examples/simple_node.rs`) pushes `value` only while ramping and never polls, so an idle cryostat looks frozen and nothing shows how a must module should poll. The Rust API has a single module interval that polls every non-constant, non-cached parameter at the same rate. Real modules need fast readback of `value` and `status` and only an occasional refresh of the rest. They also need the SECoP `pollinterval` parameter, which lets a client tune the rate at runtime. SHALL's C ABI keeps its own model (one interval for all readable parameters), which must not change.

## What Changes

- **Two polling tiers in the Rust API (opt-in).** `Module::poll_interval(fast)` now infers the tiers from the module's interface classes:
  - fast tier: `value` and `status` of a `Readable`, `Writable` or `Drivable` module, at the given interval
  - slow tier: every other non-constant, non-cached parameter, every 60 s (`Module::slow_poll_interval` changes it)
  - Both tiers start together, and nothing is polled unless the application asks for it.
- **Uniform polling stays available.** The new `Module::poll_all(interval)` polls every non-constant, non-cached parameter at one interval. `must-ffi` uses it, so the C ABI keeps SHALL's single `pollinterval` property behavior.
- **Interval precedence for each parameter:** its own interval (`Parameter::poll_interval`, SHALL's parameter `pollinterval` property) > the module's `pollinterval` parameter > the module's configured interval > no polling.
- **The SECoP `pollinterval` parameter.** If a module declares it, the library uses it to drive polling:
  - A `change` goes to the handler first; the value the handler returns (or anything later pushed) is published and re-times the module's polling.
  - The value replaces the fast-tier interval (for `poll_all`, the single interval). `0` stops that polling; the slow tier and parameters with an interval of their own keep theirs.
  - The library never polls it.
  - It drives polling only if its datainfo is a `double`, with no negative `min`. Any other `pollinterval` parameter is an ordinary parameter and gets a completion warning, so existing C hosts keep working.
- **Poll hook.** `ModuleHandler::poll(&self, ctx, due: &[&str])` receives every parameter of the module that is due at one tick, so hardware can be read in one transaction. By default it calls `read` for each parameter and publishes the results.
- **Updates for unchanged values.** The spec now states what the code already does: every published value is sent as an `update`, even if it equals the previous one.
- **Example node:**
  - polls `value`/`status` every second, with a writable `pollinterval` parameter for runtime tuning
  - adds ±5 mK of noise to the reported temperature (dev-dependency `fastrand`); status and busy logic use the true simulated temperature
  - sends `value` only through polling, no longer from the simulation loop
- **`COMPATIBILITY.md`:** a module with a `pollinterval` parameter (also one declared through the C ABI) now has its polling driven by that parameter. This is an intentional addition; SHALL treated it as an ordinary parameter.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `sec-node-runtime`: polling tiers, interval precedence, the `pollinterval` parameter at runtime, the poll hook, and updates for unchanged values.
- `sec-node-model`: declaration rules for the `pollinterval` parameter and for the module and parameter poll settings.
- `shall-server-abi`: states that SHALL's `pollinterval` property keeps uniform polling, and how a declared `pollinterval` parameter acts.

## Impact

- `must-server`:
  - `model.rs`: tier inference, `poll_all`, `slow_poll_interval`, `pollinterval` validation, and each parameter's poll source
  - `runtime/node.rs`: one poll task per module that groups the parameters due together and can be re-timed
  - `runtime/actor.rs`: `Job::Poll` carries a list of parameters and calls the hook
  - `handler.rs`: the `poll` method with a default
  - `runtime/mod.rs`: re-timing when a `pollinterval` value is published
- `must-ffi` (`server/build.rs`): `module.poll_interval` → `module.poll_all`. The C ABI behavior does not change.
- The example node, the tests that use `Module::poll_interval` (`tests/runtime.rs`, `tests/log_files.rs`), the `fastrand` dev-dependency, and `COMPATIBILITY.md`.
- API break for Rust users of `Module::poll_interval`: it changes from uniform to tiered polling. Acceptable while the crate is pre-1.0; noted in the changelog/README.
- Out of scope: tier inference for other interface classes (`Communicator`, `Acquisition*`), adaptive polling, and changing SHALL's polling mode (`SECoP_S_getStoredCommand`).
