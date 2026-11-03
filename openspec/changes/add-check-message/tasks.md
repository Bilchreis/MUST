# Tasks

## 1. Check in must-server

- [x] 1.1 Parse `check <spec> [<data>]` into `Request::Check` in `crates/must-server/src/protocol.rs`, leaving only `logging` and unknown actions in `Unsupported`. Update the parser unit test that expects `check` to be unsupported. Verify with `cargo test -p must-server protocol`.
- [x] 1.2 Add `ModuleHandler::check(&self, ctx, accessible, value: Option<Value>) -> Result<(), SecopError>`, defaulting to `NotImplemented`, to `handler.rs`. Document it with the "must not change anything, must not depend on status" rules in the trait docs and the crate docs (`lib.rs`, replacing the note that `check` is answered with `ProtocolError`). Verify that `cargo doc -p must-server` builds without warnings.
- [x] 1.3 Answer `Request::Check` in `runtime/connection.rs`: resolve, readonly, `checkable`, then validation shared with `change`/`do` (factor it out rather than copying it), then the handler under the module's request serialization and the node timeout. Reply `checked`/`error_check` only on this connection. Verify with tests in `tests/protocol.rs` for every scenario of "Check", "Check replies" and the `sec-node-runtime` delta. Replace the existing `check t1:target 5` → `ProtocolError` assertion.
- [x] 1.4 Add tests for "Empty lines" and "Logging not supported" (or confirm existing ones cover them and name them after the scenarios). Verify with `MUST_TUI=0 cargo test -p must-server`.

## 2. Check in the C ABI

- [x] 2.1 Add `action::CHECK = 4` and accept it in the filter's action range (`server/filter.rs`, `server/state.rs`). Update the unit tests that reject action 4. Verify with `cargo test -p must-ffi`.
- [x] 2.2 Store an optional check function per accessible in `server/build.rs`. Export `SECoP_S_setCheckFunction` from `server/api.rs` (focus must be a parameter or command, else `INVALID_PARAMETER`; sets or clears `checkable`). In callback mode, make `SECoP_S_nodeComplete` fail with `MISSING_MANDATORY` for checkable accessibles without a function, with a `showErrors` line naming them. Verify with unit tests for the "Check functions beyond SHALL" scenarios.
- [x] 2.3 Implement `ShallModule::check` in `server/handler.rs`: the callback on the blocking pool behind the module gate, or `request(action::CHECK, …)` in polling mode, ignoring the answer's value. Map codes with `to_secop`. Verify with the C tests of 2.5.
- [x] 2.4 Declare `SECoP_S_ACTION_CHECK` in `include/SECoP-types.h`, and `SECoP_S_checkFunction` and `SECoP_S_setCheckFunction` in `include/SECoP-exports.h`, marked "must addition, not in SHALL", with Doxygen comments in SHALL's style. Add uses to `crates/must-ffi-ctests/c/header_check.c` and `header_check.cpp`. Verify the header checks compile as C99 and C++11 and link.
- [x] 2.5 Add C tests (`c/server.c` with drivers in `tests/server.rs`) for the "Checks in callback mode" and "Checks in polling mode" scenarios, checking the client's replies over TCP. Verify they pass.
- [x] 2.6 Document check support in `COMPATIBILITY.md`: protocol section, additions to SHALL, action 4 for polling loops and the LabVIEW VIs, and `registerUserError("Impossible", …)`. Verify that the documented calls match the header.

## 3. Checks

- [ ] 3.1 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686.

## Workflow follow-up

- Archive `user-errors-and-filtered-commands` before this change (the action filter gains 4).
- Archive this change before `add-logging-messages`.
- SHALL-LabVIEW: add a case for action 4 to the VIs that dispatch stored commands (user, later).
