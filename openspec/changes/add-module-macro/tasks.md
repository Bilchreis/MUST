# Tasks

## 1. Setup

- [x] 1.1 Create the proc-macro crate `crates/must-macros` (SPDX headers, workspace lints, `syn`/`quote`/`proc-macro2` as workspace dependencies), depend on it from `must-server` and re-export it, and add `trybuild` as a dev-dependency of `must-server`. Verify with `cargo build --workspace` and `cargo deny check` passing.

## 2. Runtime start hook

- [x] 2.1 Add `ModuleHandler::start` with a no-op default, and queue a `Job::Start` in front of each module's startup reads (`runtime/node.rs`, `runtime/actor.rs`), guarded with ten times the node timeout and logged once on failure or timeout. Verify with runtime tests for *Read waits for start*, *Start hook fails* and *Hanging start hook* (paused tokio time), and with the existing `must-ffi-ctests` passing unchanged. Document the hook in the `ModuleHandler` rustdoc.

## 3. Typed values

- [x] 3.1 Implement `typed::SecopType` with `Kind`, `to_value`, `from_value` and `fits` for `f64`, `i8`–`i64`, `u8`–`u32`, `bool`, `String`, `Vec<u8>`, `Vec<T>`, tuples up to 12, and `Status` (with `Status::idle/busy/warn/error` constructors and `is_busy`). Verify with unit tests for round trips, `fits` on integer limits (*Limits too wide*) and `Status` encoding.
- [x] 3.2 Implement `#[derive(SecopEnum)]` and `#[derive(SecopStruct)]` in `must-macros` (per-field `#[datainfo(…)]`, `Option` fields as optional members). Verify with tests for *Enum datainfo* and *Optional struct member*, and trybuild failures for an enum with fields and a struct with tuple fields.

## 4. Typed module layer

- [x] 4.1 Implement `Param<T>`/`ParamHandle<T>` (shared cell, `get`, `set` with a task-local `Ctx`, else the bound `Publisher`, else store only) and `Command<A, R>`. Verify with unit tests for each publishing path.
- [x] 4.2 Define the `TypedModule` trait (declaration, `from_settings`, boxed dispatch for read/write/do, parameter binding), the `Methods` trait (which read/write methods and hooks exist, with `#[diagnostic::on_unimplemented]`), the `Settings` trait with its `serde_json::Map` implementation, `SettingsError`/`SettingsErrors`, and `typed::module::<X>(name, description, settings)`. Implement `TypedHandler<X>` (mutex, task-local scope, storing results into `Param`s, methodless parameters declared as cached, `start` order: bind → `start` method → initial writes, `poll` method). Verify with runtime tests that use a `TypedModule` written by hand, without the macro: *Value within limits reaches the method*, *Measured value out of range*, *Array too long*, *Latest read value*, *Status set while writing the target*, *Simulation task pushes the value*, *Overlapping requests*, *Connection opened in the start hook*, *Start hook fails* (typed), *Missing property*, *Unknown setting*, *Target written at start*, *Configured readonly parameter*, *Initial write fails* and *Typed and hand-written modules together*.

## 5. Module macro

- [x] 5.1 Parse the `#[module(…)]` arguments (interface class, `description`, `crate`) and the field kinds (`param!`, `command!`, `property!`, plain), and add the exported `macro_rules!` stubs. Verify with trybuild tests for *Attribute on an enum*, *Unknown flag* (and repeated/conflicting flags) and *Stray param macro*.
- [x] 5.2 Translate datainfos (key tables per SECoP type, property names as values (declaration built in `from_settings`), `status(…)`, `enum(T)`/`struct(T)`, `datainfo(expr)`, `as T` with the `KIND` const assertion and default types). Verify with tests for *Unit is published*, *Status codes*, *Default type of a double*, *Integer as u8* and *Limit and unit from properties*, and trybuild tests for *Key of another type*, *Wrong Rust type* and *Property naming a property*.
- [x] 5.3 Generate the struct rewrite, `TypedModule` (declaration, `from_settings`, dispatch with `quote_spanned!` at the declarations) and the `__must_module_<Struct>` callback macro carrying the declaration. Check the interface-class rules in the macro. Verify with runtime tests for *Drivable module*, *Declared parameter in describe*, *Command with argument and result*, *Property is not published*, *Property out of range* and *State field*, and trybuild tests for *Drivable without stop* and *Readonly target*.
- [x] 5.4 Implement `#[must_server::methods]` and the hidden `__must_methods_impl!` (dispatch, implicit callbacks, `Methods` impl, name checks with custom messages, typed calls for signatures, `start`/`poll` by name). Verify with runtime tests for *Writable parameter without methods*, *Parameter without read method is not polled* and *Helper method in the block*, and trybuild tests for *Missing methods block*, *Misspelled method*, *Write method for a readonly parameter*, *Wrong method signature*, *Command without method*, a second methods block, and a methods block in a child module (compiles).

## 6. Example and docs

- [x] 6.1 Add `crates/must-server/examples/typed_node.rs`: the simulated cryostat of `simple_node.rs` with the macro, the simulation pushing through `ParamHandle`s and noise on `value`. Verify by running it and checking with `nc`: `describe` matches `simple_node`'s accessibles, and `change cryo:target 20` ramps with `status` going BUSY then IDLE. Add it to the CI smoke test next to `simple_node`.
- [ ] 6.2 Write `docs/writing-modules.md` (declaration syntax, datainfo table with Rust types, flags, the methods block and implicit callbacks, hooks, `Param` publishing rules, background handles, construction from settings, initial values and writes, compile errors you may see) and link it in `docs/SUMMARY.md`, the README and the `must-server` crate docs. Verify with `mdbook build docs` and `cargo test --doc -p must-server`.

## 7. Integration

- [ ] 7.1 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` (including the trybuild UI tests), and cross-check the `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc` builds as CI does. Verify that CI is green on all three targets.

## Workflow follow-up

- `add-node-config-files` builds on this change: implement it after this one.
- Archive after review and merge, syncing `module-macro`, `typed-modules` and the `sec-node-runtime` delta.
- Follow-ups: faster polling while BUSY, typed modules with concurrent requests, more interface classes (`Communicator`, `Acquisition*`), and shared IO objects.
