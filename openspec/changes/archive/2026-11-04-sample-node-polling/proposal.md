# Proposal

## Why

The example node (`crates/must-server/examples/simple_node.rs`) pushes `value` only while ramping and never polls, so an idle cryostat looks frozen and nothing shows how a must module should poll. The Rust API has a single module interval that polls every non-constant, non-cached parameter at the same rate. Real modules need fast readback of `value` and `status` and only an occasional refresh of the rest. They also need the SECoP `pollinterval` parameter, which lets a client tune the rate at runtime. SECoP deprecated the `pollinterval` *properties*; the `pollinterval` parameter is the standard way, and which parameters it governs is up to the framework. C hosts that still use SHALL's properties must keep SHALL's behavior.

## What Changes

- **Polling by default, in two tiers inferred from the interface class.** Every module polls unless told otherwise (Rust API and C ABI alike):
  - fast tier: `value` and `status` of a `Readable`, `Writable` or `Drivable` module, every 5 s by default (`Module::poll_interval(fast)` changes it)
  - slow tier: every other non-constant, non-cached parameter, every 60 s (`Module::slow_poll_interval` changes it)
  - A module without one of those base classes has only the slow tier.
  - `Module::no_polling()` turns module-level polling off; parameters with an interval of their own are still polled.
- **Uniform polling stays available.** The new `Module::poll_all(interval)` polls every non-constant, non-cached parameter at one interval, without tiers. `must-ffi` uses it for SHALL's module property.
- **Interval precedence for each parameter:** its own interval (`Parameter::poll_interval`) > the module's `pollinterval` parameter (fast tier or uniform interval) > the module's configured interval > the defaults.
- **C ABI rules** (SHALL compatibility):
  - A module that declares a qualifying `pollinterval` parameter polls by the default tiers, with the parameter as the fast interval. SHALL's `pollinterval` properties on the module and its parameters are ignored, with a warning.
  - Otherwise, if any `pollinterval` property is set, SHALL's behavior applies: the module property polls all parameters at one interval, a parameter property gives that parameter its own interval and wins, and with only parameter properties only those parameters are polled.
  - With neither, the default tiers apply. This holds in callback and polling mode: a polling-mode host now receives stored `read` requests from polling.
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
- **`COMPATIBILITY.md`:**
  - A C module with a `pollinterval` parameter now has its polling driven by that parameter, and its `pollinterval` properties are ignored. SHALL treated it as an ordinary parameter.
  - A C module with neither a `pollinterval` property nor a `pollinterval` parameter is now polled by default. SHALL polled nothing; in polling mode the host now gets periodic stored `read` requests.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `sec-node-runtime`: polling tiers, interval precedence, the `pollinterval` parameter at runtime, the poll hook, and updates for unchanged values.
- `sec-node-model`: declaration rules for the `pollinterval` parameter and for the module and parameter poll settings.
- `shall-server-abi`: how a declared `pollinterval` parameter, SHALL's `pollinterval` properties, and neither decide a C module's polling.

## Impact

- `must-server`:
  - `model.rs`: tier inference, `poll_all`, `slow_poll_interval`, `pollinterval` validation, and each parameter's poll source
  - `runtime/node.rs`: one poll task per module that groups the parameters due together and can be re-timed
  - `runtime/actor.rs`: `Job::Poll` carries a list of parameters and calls the hook
  - `handler.rs`: the `poll` method with a default
  - `runtime/mod.rs`: re-timing when a `pollinterval` value is published
- `must-ffi` (`server/build.rs`): chooses between the `pollinterval` parameter, SHALL's properties (`poll_all`, `no_polling` plus parameter intervals) and the defaults, and warns about ignored properties.
- The example node, the tests that use `Module::poll_interval` (`tests/runtime.rs`, `tests/log_files.rs`), the `fastrand` dev-dependency, and `COMPATIBILITY.md`.
- API break for Rust users of `Module::poll_interval`: it changes from uniform to tiered polling. Modules without poll settings are now polled; code relying on "no polling" calls `no_polling()`. Acceptable while the crate is pre-1.0; noted in the README.
- Tests that assume no polling (handler call counts, quiet clients) set `no_polling()` or tolerate default polling.
- Out of scope: tier inference for other interface classes (`Communicator`, `Acquisition*`), adaptive polling, and changing SHALL's polling mode (`SECoP_S_getStoredCommand`).
