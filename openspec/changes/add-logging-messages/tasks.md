# Tasks

## 1. Logging request in must-server

- [x] 1.1 Parse `logging [<spec>] <level>` into `Request::Logging` in `crates/must-server/src/protocol.rs`, including the single-space form (a specifier starting with `"` is data). Update the parser unit tests. Verify with `cargo test -p must-server protocol`.
- [x] 1.2 Add `LogFilter` (node-wide level plus module overrides) to `ConnSink`, and a count of logging connections to `HubState`. Answer `Request::Logging` in `runtime/connection.rs` (resolve the module, cut parameter specifiers back to the module, validate the level, echo it). Verify with tests in `tests/protocol.rs` for every scenario of "Logging request" and "Logging levels". Replace the `error_logging` assertion in the existing test.

## 2. Log events

- [x] 2.1 Make `Hub::log` fan module records out to connections whose filter admits them (Error/Warn → `error`, Info → `info`). The line is built only when the logging count is non-zero. Push best-effort: drop when the counted bytes plus the line exceed `max_outgoing_bytes`. Add `Hub::log_debug` for debug records (tracing plus remote only). Verify with tests for the "Log events" and "Log events never disconnect" scenarios, using a handler whose poll fails for "Poll failure".
- [x] 2.2 Add `AppLogLevel` with `Ctx::log` and `Publisher::log` to `handler.rs`, and document them in the trait and crate docs (the `lib.rs` note about `logging` being unsupported goes). Verify with tests for "Handler logs" and "Debug record": the record is in the log file, or not for debug, and in `NodeEvent::Log`, or not for debug. `cargo doc -p must-server` builds without warnings.
- [x] 2.3 Update `docs/log-files.md` (application records appear; debug records do not) and `docs/status-tui.md` if it describes where records come from. Verify that the docs match the behavior of 2.2.

## 3. C ABI

- [x] 3.1 Export `SECoP_S_logMessage` from `crates/must-ffi/src/server/api.rs` (inside `guard`; node and module lookup as in `updateParameter`; level 1–4; NULL text → `INVALID_VALUE`; lossy UTF-8). Verify with unit tests for the "Unknown module", "Unknown node or node not complete" and "Invalid level or NULL text" scenarios.
- [x] 3.2 Declare `enum SECoP_S_logLevel` in `include/SECoP-types.h` and `SECoP_S_logMessage` in `include/SECoP-exports.h`, marked "must addition, not in SHALL", with Doxygen comments that note the debug routing and the slow-client effect. Add uses to `crates/must-ffi-ctests/c/header_check.c` and `header_check.cpp`. Verify the header checks compile as C99 and C++11 and link.
- [x] 3.3 Add C tests (`c/server.c` with drivers in `tests/server.rs`) for "Host logs for a module" and "From any thread", checking the `log` events over TCP. Verify they pass.
- [x] 3.4 Document remote logging and `SECoP_S_logMessage` in `COMPATIBILITY.md` (protocol section, additions to SHALL, level mapping, node-wide records not forwarded). Verify that the documented calls match the header.

## 4. Checks

- [ ] 4.1 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686.

## Workflow follow-up

- Archive `add-check-message` before this change (it adds the "Logging not supported" requirement removed here).
