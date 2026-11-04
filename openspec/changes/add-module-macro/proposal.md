# Proposal

## Why

Writing a MUST module means building its description by hand (`Module`, `Parameter`, `DataInfo` builders) and then matching parameter names in a `ModuleHandler` that converts `Value`s back and forth (`examples/simple_node.rs`). The declaration and the behaviour live in two places and are linked only by strings. Frappy lets a driver declare its parameters next to each other and implement `read_<p>`/`write_<p>` methods. secop-rs shows that a Rust derive macro can do the same, but it uses datainfo strings, hard-codes `Drivable` as the interface class, and requires plumbing fields (comparison in issue #59). Module types for node config files (`add-node-config-files`) need a declaration from which the description, the dispatch and the config settings all come.

## What Changes

- **Module declaration macro.** An attribute macro `#[must_server::module(...)]` on a struct names the interface class (`Readable`, `Writable`, `Drivable`, or none) and the default description. Inside the struct, one line per accessible:
  - `value: param!("current temperature", double(min = 0.0, max = 1500.0, unit = "K"))`, with flags such as `writable`, `cached`, `default = …`, `group`, `visibility`, `checkable` and `constant = …`
  - `stop: command!("stop at the current temperature")`, with optional `argument = …` and `result = …`
  - `heater_loop: property!("heater loop", int(min = 1, max = 2), default = 1)` for config-only settings that are not SECoP accessibles
  - plain fields with no macro for the module's own state (`Default`-initialized)
- **Datainfos in SECoP's own names and keys**, written as Rust tokens: `double`, `scaled`, `int`, `bool`, `enum(Type)`, `string`, `blob`, `array(…)`, `tuple(…)`, `struct(Type)`, and `status(IDLE, BUSY, …)`. A datainfo value can name a property of the module (`double(max = range, unit = unit)`), so each instance gets its own limits and unit. The Rust type of a field comes from its datainfo (`double` gives `f64`, `int` gives `i64`), and `as T` picks another, e.g. `int(min = 1, max = 2) as u8`. Unknown datainfo keys, a datainfo that doesn't fit the Rust type, and a missing accessible required by the interface class (`value`, `status`, a writable `target`, `stop`) are compile errors at the offending token.
- **Behaviour by naming convention, as in Frappy:**
  - `async fn read_<p>(&mut self) -> Result<T>` for each parameter that is neither cached nor constant
  - `async fn write_<p>(&mut self, v: T) -> Result<T>` for each writable, non-cached parameter, returning the value actually set
  - `async fn do_<c>(&mut self[, arg]) -> Result<R>` for each command
  
  A missing or mistyped method is a compile error. Optional `start` and `poll` hooks.
- **Typed runtime layer** (`must_server::typed`), on top of the existing `Module`/`ModuleHandler`, usable without the macro:
  - `SecopType`, the mapping between Rust types and `Value`/`DataInfo`. It covers `f64`, the integer types, `bool`, `String`, `Vec<u8>`, `Vec<T>`, tuples and `Status`, plus `#[derive(SecopEnum)]` and `#[derive(SecopStruct)]`.
  - `Param<T>`. `get()` returns the latest value. `set(v)` publishes an update: inside a request it reaches clients before the reply, as `Ctx::update` does. `handle()` gives a cloneable handle for background tasks.
  - an adapter that serves a typed module as a `ModuleHandler`, one request at a time with `&mut self`
  - construction from settings (properties, configured initial values) with typed errors
- **Configured initial values.** A configured value of a cached parameter sets its cache. A configured value of a writable hardware parameter is written through `write_<p>` when the node starts, as Frappy and secop-rs do.
- **`must-server` runtime: a module start hook.** `ModuleHandler::start` runs once per module before its startup reads and polling. The typed layer uses it for the `start` hook and the initial writes. Existing handlers get a no-op default.
- **Example:** `examples/typed_node.rs`, the simulated cryostat of `simple_node.rs` written with the macro.

## Capabilities

### New Capabilities
- `module-macro`: the `#[must_server::module]` declaration syntax (`param!`, `command!`, `property!`, datainfo syntax, `as T`), the naming convention for behaviour methods, and the compile-time checks.
- `typed-modules`: typed values (`SecopType` and the derives), `Param<T>`, serving a typed module (one request at a time, publishing, hooks), and construction from settings, including configured initial values and initial writes.

### Modified Capabilities
- `sec-node-runtime`: a per-module start hook, run before the module's startup reads and polling.

## Impact

- New proc-macro crate `crates/must-macros` (`syn`, `quote`, `proc-macro2`, all already in `Cargo.lock`), re-exported by `must-server`. New dev-dependency `trybuild` for compile-fail tests. `cargo deny` must pass.
- `must-server`:
  - a new `typed` module
  - `ModuleHandler::start` (`handler.rs`), run first in each module actor (`runtime/node.rs`, `runtime/actor.rs`)
  - re-exports of `module`, `param!`, `command!`, `property!` and the derives
- `must-types`: no change. The typed layer uses the existing `DataInfo` builders and `Value`.
- Rust only. The C ABI (`must-ffi`) and its handlers are unaffected: they get the no-op `start`.
- `add-node-config-files` builds on this change: module types become macro-declared structs, and config settings become properties and initial values. It is revised alongside.
- **Out of scope:**
  - modules with concurrent requests through the macro (`&mut self` means one request at a time)
  - shared IO objects
  - interface classes beyond `Readable`/`Writable`/`Drivable` (e.g. `Communicator`, `Acquisition*`)
  - the `matrix` datainfo in the macro
  - faster polling while BUSY (a separate runtime change)
  - inheritance-style reuse of behaviour between module types (use composition and helper structs)
