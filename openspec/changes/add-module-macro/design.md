# Design

## Context

- A module today is a `must_server::Module` built with `Parameter`/`Command` and `DataInfo` builders (`DataInfo::double().min(…).unit(…)`, generic `.prop(key, value)`), plus a `ModuleHandler` (`read`/`change`/`call`/`check`/`poll`, `async_trait`, `&self`) that matches parameter names and converts `must_types::Value`.
- The module actor (`runtime/actor.rs`) calls the handler for one request at a time unless `concurrent_requests` is set. `Ctx::update` publishes so that the update reaches clients before the request's reply. `Ctx::publisher()` gives a `Publisher` for background tasks. Startup reads and polling are queued as jobs (`runtime/node.rs`).
- `DataInfo::decode(json, Mode::Request)` checks a value with all limits. `Mode::Report` checks only the shape.
- There is no hook that runs when a node starts.
- `syn`, `quote` and `proc-macro2` are already in `Cargo.lock` (through `async-trait` and `thiserror`).
- secop-rs (GPL-2.0+) is prior art for the approach. We take ideas only, no code.

## Goals / Non-Goals

**Goals:**
- One line per accessible, readable without knowing Rust macros: description, datainfo in SECoP terms, flags.
- Every declaration mistake that can be found at compile time is a compile error at the offending token.
- Generated code uses only the public `must-server` API, so the typed layer is an ordinary library that can be used, and tested, without the macro.

**Non-Goals:**
- Changing the protocol behaviour of a node. A typed module and a hand-written module with the same declaration behave the same on the wire.
- Inheritance between module types. Shared behaviour is shared through helper structs, functions or generics.

## Decisions

### Declaration: `param!` in field-type position, read by the outer attribute

```rust
#[must_server::module(Drivable, description = "LakeShore temperature loop")]
pub struct TemperatureLoop {
    value:  param!("current temperature", double(min = 0.0, max = 1500.0, unit = "K")),
    status: param!("current status", status(IDLE, WARN, BUSY, ERROR)),
    target: param!("setpoint", double(min = 0.0, max = 1500.0, unit = "K"), writable),
    tolerance: param!("convergence criterion", double(min = 0.0, unit = "K"), writable, default = 0.1),
    pollinterval: param!("polling interval", double(min = 0.1, max = 120.0, unit = "s"), writable, default = 5.0),
    stop: command!("stop at the current temperature"),

    uri: property!("controller address", string()),
    channel: property!("sensor channel", string(minchars = 1, maxchars = 1)),
    heater_loop: property!("heater loop", int(min = 1, max = 2) as u8, default = 1),
    heater_range: property!("heater range", int(min = 0, max = 5) as u8),

    conn: Option<LineConnection>,   // own state, Default-initialized
}

#[must_server::methods]
impl TemperatureLoop {
    async fn start(&mut self) -> Result<()> { … }                    // hook: connect using `uri`
    async fn read_value(&mut self) -> Result<f64> { … }
    async fn read_status(&mut self) -> Result<Status> { … }
    async fn write_target(&mut self, target: f64) -> Result<f64> { … } // no read_target: stored value
    async fn do_stop(&mut self) -> Result<()> { … }
    // tolerance, pollinterval: no methods, implicit callbacks
}
```

- Rust parses a macro call in type position. Attribute macros expand before the macro calls inside their item, so `#[module]` receives the raw `param!(…)` tokens, takes the declaration out of them and replaces each field's type: `Param<f64>`, `Command<(), ()>`, `u8`.
- `param!`, `command!` and `property!` also exist as exported `macro_rules!` that expand to `compile_error!("… only valid in a #[must_server::module] struct")`. This keeps imports resolvable for rust-analyzer and covers the *stray macro* scenario.
- The arguments are valid Rust expressions (`double(min = 0.0)` parses as a call with assignment arguments), so rustfmt formats the lines.
- *Alternatives considered:*
  - field attributes plus an explicit field type (`#[param(…)] value: Param<f64>`), which repeats the type
  - datainfo strings (secop-rs), which give no token-precise errors
  - one function-like macro for the whole module, which rustfmt and rust-analyzer don't handle inside
  - datainfos as types, which can't carry `f64` limits on stable Rust

  The user chose this syntax in the issue #59 discussion.

### Behaviour: one `#[must_server::methods]` block, no interface-class traits

Earlier sketches used `trait Drivable { async fn read_value(…); … }`. Since all accessibles, `value`/`target`/`stop` included, are now declared explicitly, such a trait would only repeat their signatures. Behaviour methods are named as in Frappy and secop-rs (`read_<p>`, `write_<p>`, `do_<c>`, plus the hooks `start` and `poll`) and live in one impl block with `#[must_server::methods]`.

**Why a second attribute:** Frappy's rule, that a parameter without `read_<p>` is served from its stored value and one without `write_<p>` stores the change, needs to know which methods exist. The struct attribute can't see impl blocks. Annotating the methods block gives the macros that knowledge at compile time:
- methodless parameters are known statically, so they get no polling and no startup reads;
- the config rules (*Configured initial values*) are exact;
- misspelled method names can be reported.

**How the two macros meet (callback macro):**
1. `#[module]` emits, next to the struct, a `macro_rules! __must_module_<Struct>` whose body holds the struct's whole declaration as tokens, made path-addressable with `pub(crate) use`.
2. `#[methods]` doesn't generate code itself. It expands to an invocation of that macro (same path as the impl's self type, last segment replaced) with the impl block's tokens.
3. The `macro_rules!` forwards both token sets to a hidden proc macro `__must_methods_impl!`. That macro sees the declaration and the methods, and generates:
   - the dispatch;
   - the implicit callbacks;
   - `impl typed::Methods for X`, with which parameters have read and write methods and whether `start`/`poll` exist;
   - every check, with its own message at the method's span: unknown `read_vlaue`, `write_` of a readonly parameter, a missing `do_<c>`.
   
   Signatures are checked by typed calls (`let v: f64 = …` / `write_target(v: f64)`), so a mismatch is rustc's type error, spanned to the method.
4. **Missing or duplicate methods block.** The `TypedModule` impl from `#[module]` requires `X: typed::Methods`. That trait carries `#[diagnostic::on_unimplemented(message = "`{Self}` needs a `#[must_server::methods]` impl block")]`, so a missing block gets that message. Two blocks give conflicting `Methods` impls, which the docs explain.
5. **Placement.** The methods block must be in the same crate as the struct, and name the struct by a path through which the struct's module is reachable. That's the normal case, and it's documented.

**Other rules:**
- **Interface classes.** The interface-class rules (required accessibles, writable `target`, `stop` without argument, `BUSY` in `status`) are checked by `#[module]`, which sees all declarations. `NodeBuilder::build` keeps checking them at runtime too, for hand-written modules.
- **Hooks are found by name** in the block. There is no `hooks` flag and no `Hooks` trait.
- **Prefixed helpers.** A helper with a `read_`/`write_`/`do_` name that isn't an accessible (e.g. `read_raw`) goes in another, unannotated impl block.

*Alternatives considered:*
- **An explicit `cached` flag, with a missing method as a compile error.** Rejected: it doesn't follow Frappy's rule, which the user asked for.
- **The "inherent method shadows trait method" trick:** generate a trait with default `read_<p>` methods and let a user's inherent method win. It needs no second attribute, but a typo silently falls back to the stored value, and which methods exist is only known at runtime.

### Datainfo translation

The macro translates each datainfo to `must_types` builder calls: `double(min = 0.0, unit = "K")` becomes `DataInfo::double().prop("min", 0.0).prop("unit", "K").build()`.
- **Keys:** the macro has a table of allowed keys per SECoP type (`double`: `min`, `max`, `unit`, `fmtstr`, `absolute_resolution`, `relative_resolution`; `string`: `minchars`, `maxchars`, `isUTF8`; …). Unknown keys are compile errors. Values are arbitrary expressions, so constants work.
- **`status(…)`** expands to a tuple of an enum with the named SECoP status codes (`DISABLED` 0, `IDLE` 100, `STANDBY` 130, `PREPARED` 150, `WARN` 200, …, `BUSY` 300, …, `ERROR` 400, …) and a string.
- **`enum(T)` and `struct(T)`** call `T::datainfo()` from the derives.
- **`datainfo(expr)`** passes a `DataInfo` through, and requires `as T`.

**Properties in datainfo values:** a datainfo value may be the name of a property of the module, e.g. `double(max = range, unit = unit)` (spec: *Datainfos from properties*). The macro sees which identifiers are property names. When any datainfo uses one, it moves building the declaration from a static into the generated `from_settings`: there, after the properties are read and checked, each property is bound as a local (`let range: f64 = …;`) before the datainfos are built. A property's own datainfo naming a property is rejected, because properties are read first and in no particular dependency order. This gives each instance its own limits and unit without SECoP dynamic limits, which is what `must::sim::FlowController` in `add-node-config-files` needs.

Datainfo values that are runtime expressions are checked by `DataInfo` building when the module is constructed. Errors such as `min > max` therefore surface as construction errors naming the accessible, not as compile errors.

### Rust types: `SecopType`

```rust
pub trait SecopType: Sized + Clone + Send + 'static {
    const KIND: Kind;                       // Double, Int, Bool, String, Blob, Array, Tuple, Enum, Struct, Status
    fn to_value(&self) -> Value;
    fn from_value(v: &Value) -> Result<Self, SecopError>;
    fn fits(datainfo: &DataInfo) -> Result<(), String> { Ok(()) } // e.g. int limits within u8
}
```

- **Implementations:** `f64`, `i8`…`i64`, `u8`…`u32` (`u64` doesn't fit SECoP's 64-bit signed `int` everywhere), `bool`, `String`, `Vec<u8>` (blob), `Vec<T>` (array), tuples up to 12, `Status`, and the derives.
- **Type checks:** the macro emits `const _: () = assert!(<T as SecopType>::KIND as u8 == Kind::Double as u8, …)` at the `as T` span, so a wrong type is a compile error (*Wrong Rust type*). `fits` runs at construction (*Limits too wide*). Without `as`, the macro picks the default type for the datainfo.
- **`Vec<u8>` is always a blob.** An `array(int(…) as u8)` therefore needs a newtype. This is rare, and it's documented.
- **Derives:** `#[derive(SecopEnum)]` (field-less enum, discriminants as codes, variant names as member names) and `#[derive(SecopStruct)]` (named fields; per-field `#[datainfo(…)]` uses the same syntax; `Option<T>` makes a member optional).

### `Param<T>` and publishing

`Param<T>` wraps an `Arc<ParamCell<T>>` holding the parameter name, its datainfo, a `std::sync::Mutex<T>` and a `OnceLock<Publisher>`.
- **`get()`** clones the stored value. Every read result, write result, cache hit and value pushed through a handle is stored, so `get` always returns the latest value (*Latest read value*).
- **`set(v)`** stores, then publishes:
  - inside a request it goes through the request's `Ctx`, found in a `tokio::task_local!` that the adapter sets around each method call, so the update comes before the reply (*Status set while writing the target*);
  - otherwise, after start, it goes through the bound `Publisher`;
  - before start, it only stores, and the value becomes the initial cache value.
- **`handle()`** returns a cloneable `ParamHandle<T>` that shares the cell. Spawned tasks don't inherit the task-local, so they always publish through the `Publisher`.
- *Alternative considered:* collect `set` calls and flush them after the method returns. Rejected because a long command (e.g. a 10 s `do_wait` that sets `status` along the way) would publish only at the end, unlike Frappy.

### Adapter: `TypedHandler<X>`

- **Locking:** `TypedHandler<X: TypedModule>` holds `tokio::sync::Mutex<X>` and implements `ModuleHandler`. Every handler method locks, runs the generated dispatch inside the task-local scope, converts with `SecopType`, stores into the `Param` and returns the `Reading`. Since the module is not `concurrent_requests`, the actor already serializes requests, so the lock is never contended. It only provides `&mut self` safely.
- **Send futures:** the generated dispatch boxes the user's futures as `Pin<Box<dyn Future + Send + '_>>`. A non-`Send` future (e.g. a `std` `MutexGuard` held across an `.await`) is reported at the generated call, which is spanned to the method. This is documented.
- **`poll`:** without a `poll` method it is not overridden, so the default (`ctx.read` per due parameter) keeps shared-read semantics. With one, it locks and calls it, and it publishes through `set`.
- **Implicit callbacks:** parameters without a read method are declared to the `Module` as cached (no polling, no startup reads, served from the cache). A `change` of a writable parameter without a write method is answered by the generated dispatch, which returns the value so the adapter stores and publishes it.
- **`start`** runs in this order: bind every `Param`'s `Publisher` → the `start` method, if any → the initial writes in declaration order (`write_<p>(configured)`, the result stored and published, a failure logged and published with `Ctx::update_error`) → return. Configured values of readonly parameters without a read method are set as `Parameter::initial_value` at construction, so no work is needed at start.

### Construction: `Settings`

```rust
pub trait Settings {
    fn take(&mut self, key: &str) -> Option<Json>;          // marks the key consumed
    fn remaining(&self) -> Vec<String>;
    fn error(&self, key: Option<&str>, message: String) -> SettingsError; // located by the implementation
}
```

- **Generated constructor:** `X::from_settings(&mut dyn Settings)` takes each property (with its default), checks it with `DataInfo::decode(…, Mode::Request)` and converts it. It takes the configured values of parameters, applies the rules of *Configured initial values*, and reports `remaining()` as unknown settings. Errors are collected, not returned at the first one.
- **Implementations:** `Settings` is implemented for `serde_json::Map<String, Json>` for code-only nodes: `typed::module::<X>("T", "sample temperature", json!({"channel": "A", …}))` returns a `Module`. `must-config` implements it over its spanned tree, so errors carry file and line.
- **Why JSON values:** they make every value go through the same `DataInfo` check that `change` uses, so a config value and a client value are checked alike.

### Runtime: `ModuleHandler::start`

- **Interface:** `async fn start(&self, ctx: &Ctx) -> Result<(), SecopError>` with a no-op default.
- **Runtime:** `start_polling` (`runtime/node.rs`) queues a `Job::Start` in front of the startup reads. The actor runs it like other jobs, through `guarded`, but with a limit of ten times the node timeout (connecting to hardware may take longer than a request), and logs failure or timeout once.
- **Ordering:** requests that arrive meanwhile wait in the actor queue, so no request reaches the handler before `start` has finished.
- **C ABI:** `must-ffi` handlers get the default, so nothing changes for C hosts.

### Crates and paths

- **`crates/must-macros`** (proc-macro) holds `module`, `SecopEnum` and `SecopStruct`. `must-server` re-exports them, with the `macro_rules!` stubs, as `must_server::{module, param, command, property, SecopEnum, SecopStruct}`, and `must_server::typed::prelude` for the rest.
- **Generated paths:** generated code refers to `::must_server::…`. A crate that renames the dependency needs `#[must_server::module(crate = path)]`, as serde's `crate` option works.

### Testing

- **Compile errors:** `trybuild` tests in `must-server/tests/ui/`, one per compile-error scenario, with `.stderr` snapshots. The pinned toolchain (`rust-toolchain.toml`) keeps them stable, and a toolchain bump updates them with `TRYBUILD=overwrite`.
- **Runtime:** runtime tests with the existing helpers (`tests/common`) serve typed modules and drive them over TCP.

## Risks / Trade-offs

- [Proc-macro errors can be cryptic] → Every generated item is spanned to its declaration, the macro checks what it can before emitting code, and the UI tests pin the messages.
- [Implicit callbacks hide a forgotten `read_<p>`: the parameter quietly serves its stored value] → This is Frappy's rule, and what the user asked for. Misspelled names are still errors, and the docs and `describe` debugging show which parameters are served from the cache. A later `must check --verbose` could list them.
- [The callback-macro handshake is unusual: a `macro_rules!` per module type, and path rules for where the methods block may live] → Generated names are prefixed `__must_`. The placement rule matches how modules are normally written. The UI tests cover a block in a child module and the *Missing methods block* message.
- [`&mut self` rules out concurrent requests for typed modules] → Accepted for now. A later `&self` variant with interior mutability can add `concurrent_requests`.
- [Task-local `Ctx` doesn't reach tasks spawned inside a method] → They publish through `Publisher` like any background task. The ordering guarantee applies only to the method's own task, which is documented.
- [Generated code increases compile time] → One proc-macro crate with no heavy dependencies beyond `syn`.

## Migration Plan

Additive. Hand-written `Module`/`ModuleHandler` code keeps working unchanged. `ModuleHandler::start` has a default, so existing implementors compile as before. `examples/simple_node.rs` stays, and `examples/typed_node.rs` shows the same cryostat with the macro.

## Open Questions

- Whether the attribute should be spelled `module` or `secop_module`. `module` reads well but is a common word. This is decided during review, and is a rename only.
