# Design

## Context

Polling today (`must-server`):
- `Module::poll_interval(d)` applies to every parameter that is neither constant nor cached and has no interval of its own (`model.rs`, `finish_module`). `Parameter::poll_interval(d)` overrides it.
- `start_polling` (`runtime/node.rs`) spawns one `tokio::time::interval` task per polled parameter. Each tick sends `Job::Poll { parameter }` to the module actor with `try_send`, so a tick is skipped while the backlog of 64 is full. The actor calls `handler.read` and publishes the result.
- `Hub::store` sends every published value to activated clients and does not compare it with the cached one. "Update even if unchanged" is already the behavior and only needs to be specified and tested.
- `must-ffi` maps SHALL's module and parameter `pollinterval` properties onto these two calls (`server/build.rs`, `set_poll`; 0 means none, capped at 3600 s).

The example node pushes `value` from its simulation loop only while ramping and has no polling.

## Goals / Non-Goals

**Goals:**
- Sensible polling for every module by default, inferred from its interface class, in the Rust API and the C ABI.
- Runtime tuning through SECoP's predefined `pollinterval` parameter, with the hardware handler in control.
- Batched hardware access for parameters due at the same time.
- C hosts that use SHALL's `pollinterval` properties keep SHALL's behavior; the deprecated properties give way to the standard `pollinterval` parameter when both are present.

**Non-Goals:**
- Tiers for `Communicator` and the `Acquisition*` classes. Their parameters land in the slow tier.
- Adaptive polling (e.g. faster while BUSY) or per-client rates.
- Changing how SHALL's polling mode (`SECoP_S_getStoredCommand`) carries a poll: each poll still becomes a stored `read` request.
- A must-specific marker for which parameters belong to the fast tier. For now the interface class decides; a marker can be added later.

## Decisions

### API shape
- **Default (no call):** tiered. Fast tier = `value` + `status` if the base class (last `interface_classes` entry) is `Readable`, `Writable` or `Drivable`, every 5 s (`DEFAULT_FAST_POLL`). Slow tier = every other non-constant, non-cached parameter except `pollinterval`, every 60 s. 5 s is Frappy's default `pollinterval`.
- `Module::poll_interval(fast)`: tiered with another fast interval.
- `Module::slow_poll_interval(d)`: changes the slow tier from its 60 s default. It has an effect only with tiers (warning otherwise). The slow tier is always on when the fast tier is.
- `Module::poll_all(d)`: uniform polling, the old meaning of `poll_interval`. `must-ffi` uses it for SHALL's module property.
- `Module::no_polling()`: no module-level polling; only parameters with an interval of their own are polled. `must-ffi` uses it when only parameter properties are set; Rust code and tests that need a quiet module use it too.
- Setting more than one of `poll_interval`, `poll_all` and `no_polling` on one module is a completion error. It is not a silent "last call wins", because they mean different things.
- Internally the module keeps the optional tiered and uniform intervals, a polling-off flag and the optional slow interval; the combination check counts how many of the first three were set.
- *Alternative considered:* keep `poll_interval` uniform and add `poll_tiered`. Rejected because the common case should get the short name, and the crate is pre-1.0. The break is caught at review time: every existing Rust caller is in this repo (two tests and `must-ffi`).

### Model: poll source per parameter
`finish_module` resolves each parameter to a `PollSource`, kept in `ParameterModel` instead of `poll_interval: Option<Duration>`:
```rust
enum PollSource { None, Own(Duration), Fast, Slow }
```
The module model keeps `fast: Option<Duration>` (the tiered fast interval, configured or 5 s, or the uniform interval; `None` with polling off; under `poll_all` every polled parameter is `Fast`), `slow: Duration`, and `driven: bool`, which is true when a qualifying `pollinterval` parameter exists and polling is not off. The effective interval is resolved at runtime:
- `Own(d)` → `d`
- `Fast` → current `pollinterval` value if the parameter drives the module, else `fast`
- `Slow` → `slow`
- `None` → no polling (constant, cached, `pollinterval` itself, or polling off without an own interval)

This implements the precedence rule: parameter interval > module `pollinterval` parameter > module configuration > defaults. Own intervals on constant or cached parameters or on `pollinterval` become `None` with a warning. They are not errors, because SHALL accepted them.

### Runtime: one poll task per module
The per-parameter tasks are replaced by one task per module that has any polled parameter:
- It keeps a `next_due: Instant` per parameter and sleeps until the earliest one (`tokio::time::sleep_until`).
- At wake-up it collects every parameter whose due time has passed, sends `Job::Poll { parameters }` with `try_send`, and advances each one's due time by its interval. Like today's `MissedTickBehavior::Delay`, this means `now + interval` after a late tick, so there is no burst.
- Parameters that share an interval and start time therefore fall due together. This is what makes the poll hook useful, e.g. `value` + `status`, and every 60th tick also the slow tier.
- Re-timing: the task also selects on a `watch::Receiver<Duration>` for the module's fast interval. A new value updates the `Fast` parameters' interval and sets their next due time to `now + new` (`now` immediately for a shorter interval would also do, but `now + new` keeps the logic simple and is within one old interval). `0` sets them to "never" until a positive value arrives. The task stays alive while only `Own`/`Slow` parameters remain.
- The startup reads are unchanged (per-parameter `Job::Poll` through `read`), so a module without a poll hook behaves as before at startup. The fast tier's first tick is one interval after start, as today.

### `pollinterval` parameter
- **Change path:** handler-first, so there is no special case in dispatch. `Job::Change` calls `handler.change`, then `publish_value`. `publish_value` (in `Hub`) checks whether the parameter is the module's qualifying `pollinterval`. If so, it sends the published double (clamped to at most 3600 s, like SHALL's cap; a negative value is impossible because of the datainfo) to the module's `watch::Sender`.
- **Same path for pushes:** application pushes (`RunningNode::update`, `Ctx::update`) take the same path, so "the library applies published values" holds for every source. A failed change publishes nothing and leaves the interval alone.
- **Initial value:** the cache starts with `initial_value` if given (and the watch with it), else with the module's `fast` interval as a double (5.0 by default). With polling off, the parameter drives nothing and gets a warning. The parameter is excluded from startup reads and polls. A client `read` of it still goes to the handler, like any non-cached parameter. A handler that does not implement `read` for it should mark it `cached()`. The example does that.
- **Default handler for `change`:** none. A module that wants the library to just accept the value implements `change` as `Ok(value.into())`. This keeps "handler-first" literal and leaves hardware minimums to the handler.

### Poll hook
```rust
async fn poll(&self, ctx: &Ctx, due: &[&str]) {
    for p in due {
        match self.read(ctx, p).await {
            Ok(r) => { let _ = ctx.update(p, r); }
            Err(e) => ctx.update_error(p, e),
        }
    }
}
```
- `run_job` for `Job::Poll { parameters }` builds the `&[&str]` from the owned `Vec<String>` inside the guarded task, since `guarded` needs `'static`.
- The whole hook runs under the node timeout. On timeout or panic the error is logged once for the module. No `error_update` is sent for the due parameters: the hook may already have published some of them, and the next tick retries.
- The default hook keeps today's warning log for each failed read.
- The ordering guarantee is unchanged: polls are serialized with requests in the module actor.

### C ABI
`build.rs` decides per module, after all accessibles are known (in `build_module`):
1. **Qualifying `pollinterval` parameter** (same test as the model: `double` datainfo, no negative `min`): no poll call on the module and no `Parameter::poll_interval`, so the default tiers apply with the parameter as fast interval. Each ignored `pollinterval` property (module or parameter) becomes a completion note, like SHALL's filled-in defaults: `SECoP_S_nodeComplete` returns a warning code and `SECoP_S_showErrors` prints the text, which says the property is ignored because of the parameter. The setter callback (or polling-mode request) answers changes of the parameter first.
2. **Any `pollinterval` property:** the module property gives `module.poll_all(d)` (0 means none, so `no_polling()`); without a module property, `module.no_polling()`. Parameter properties give `Parameter::poll_interval(d)`. This is SHALL's behavior, as before this change.
3. **Neither:** no poll call; the default tiers apply.

In polling mode a poll is a stored `read` request like a client's, answered through `SECoP_S_putCommandAnswer`. Default polling therefore hands such hosts a `read` of `value` and `status` every 5 s and of the rest every 60 s. A host that never answers has each poll fail with `TimeoutError` after the node timeout, logged, and the module is busy meanwhile. This is the accepted cost of one default for both modes; it is documented in `COMPATIBILITY.md`.

### Example node
- `cryo`:
  - `.poll_interval(1 s)`
  - a writable, cached `pollinterval` parameter (`double`, `min` 0, `max` 3600, unit `s`, initial 1.0) whose `change` the handler accepts as is
  - `target`/`ramp` slow-tier polled every 60 s
- **Noise:** `read("value")` returns `temperature + fastrand::f64() * 0.01 - 0.005` (±5 mK). `status` and `busy()` use the noiseless `temperature`. `fastrand` is a `[dev-dependencies]` entry of `must-server`, since examples use dev-dependencies, added to `[workspace.dependencies]` following the workspace convention. It is already in `Cargo.lock`.
- **Simulation loop:** it only steps the simulation and pushes `status` on busy/idle transitions (so a client sees the transition at once). `value` reaches clients only through polling.
- **Docs:** the module doc comment shows `change cryo:pollinterval 0.2`.

## Risks / Trade-offs

- [Rust API break of `Module::poll_interval`] → all callers are in-repo and updated in this change; noted in the README/changelog.
- [C hosts with a `pollinterval` parameter change behavior] → only for `double` parameters with that exact SECoP-reserved name; their `pollinterval` properties are ignored with a warning; documented in `COMPATIBILITY.md`; the change still goes through their setter first.
- [C hosts without any poll configuration are now polled] → getters are called every 5 s/60 s, and polling-mode hosts receive stored `read` requests they did not get from SHALL. Hosts that handle client reads already handle these. Documented in `COMPATIBILITY.md`; a host restores SHALL's "nothing polled" by setting the module property `pollinterval` 0.
- [Rust modules are now polled by default] → handlers without `read` for some parameter log a failed poll every 60 s; tests and applications that need a quiet module call `no_polling()`.
- [A hook override that forgets to publish] → no updates for those parameters. This is documented on the trait. There is no runtime detection, because "publish nothing" is a valid choice for hardware that did not answer.
- [Unchanged-value updates cost traffic and log volume] → this is today's behavior. Fast tiers at 1 s are within the log-files disk estimate. Clients rely on regular updates as a liveness signal (SECoP practice).
- [One task per module instead of per parameter] → a slow hook delays the module's other due parameters. That already happens today through the actor, so nothing is lost.

## Migration Plan

This lands as one change. In-repo callers move from `poll_interval` (uniform) to `poll_all`, or deliberately to tiered `poll_interval` (example node). Tests that count handler calls or expect a quiet connection get `no_polling()` where default polling would disturb them. There is no persisted state.

## Open Questions

- Whether `pollinterval = 0` should also stop the slow tier. Assumed **no**: the parameter governs the fast tier and the uniform interval only. This is easy to revisit.
- Resolved: a `pollinterval` parameter on a module without other poll configuration uses the default tiers (it was uniform from 0 before).
