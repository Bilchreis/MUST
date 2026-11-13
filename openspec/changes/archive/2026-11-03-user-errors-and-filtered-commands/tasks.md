# Tasks

## 1. Custom error classes in must-types

- [x] 1.1 Add `custom_class: Option<Box<str>>` and `SecopError::custom(name, message)` to `crates/must-types/src/error.rs` (a standard class name gives that class without override; otherwise `class` is `InternalError`; debug assertion on a name that is not a SECoP identifier) and make `report::error_report` send the custom name; verify with unit tests for both cases and the "Custom error class" report shape
- [x] 1.2 Document custom error classes on `SecopError` and in the `must-server` crate docs (handlers return them like any `SecopError`); verify with a runtime test in `must-server` that a read handler's custom error reaches the client as `error_read t1:value ["_Overheated", ...]`, and `cargo doc` builds without warnings

## 2. User error codes in must-ffi

- [x] 2.1 Add the registry to `crates/must-ffi/src/server/codes.rs` (own mutex, codes from -1000 down and never reused, count per code, case-insensitive name index including the SECoP 2.0 class names, `CString`s kept for the process lifetime) with `register`, `unregister` and lookup; verify with unit tests for new code, same name again (same code, new text), case clash with a registered or a standard name, invalid names, count down to removal, no reuse after removal, unknown codes including SHALL's
- [x] 2.2 Make `description`, `error_class` and `to_secop` consult the registry for codes ≤ -1000 (`to_secop` builds `SecopError::custom(name, "{accessible}: {text}")`); verify with unit tests, and that the existing `every_code_has_a_description` test still passes
- [x] 2.3 Export `SECoP_S_registerUserError` and `SECoP_S_unregisterUserError` from `server/api.rs` (inside `guard`, no `initLibrary` needed) and let `SECoP_S_getErrorString` return the registry's string for registered codes; verify with a unit test that the pointer stays valid and unchanged after unregistering
- [x] 2.4 Add C tests to `crates/must-ffi-ctests` (`c/server.c` with drivers in `tests/server.rs`) for the scenarios "Register and answer" (polling mode, `putCommandAnswer2`), "From a callback", "Standard class name", "Unregister" and "Registered twice, unregistered once", checking the client's error reports over TCP

## 3. Filtered polled requests in must-ffi

- [x] 3.1 Add the filter parser (split at `,`, `;` and ASCII whitespace; 1–3 parts each checked with `validate_identifier`; NULL/empty matches all; action 0–3) and matching (parts equal the leading parts of `node:module:accessible`, ignoring ASCII case); verify with unit tests for the "Filter syntax" scenarios including the tab-and-line-end list, too many parts and bad identifiers
- [x] 3.2 Add `take_matching(context, filter)` to `server/state.rs`: remove the matching request with the lowest id from that context's queue and move it to `handed_out`; verify with unit tests that non-matching requests stay queued in order and that other contexts' queues are untouched
- [x] 3.3 Export `SECoP_S_filterStoredCommand` and `SECoP_S_filterStoredCommand2` from `server/api.rs` with the VI argument order (filter first) plus a trailing `szContextID` read with `context_id`, sharing the output code of `getStoredCommand[2]` (`write_text`, new value handle, buffer warning); return `INVALID_CONTEXT_ID` for an unknown context, `INVALID_NAME`/`INVALID_VALUE` for a bad filter/action and `NO_DATA` when nothing matches; verify with the C tests of 3.4
- [x] 3.4 Add C tests for "Only the selected accessibles", "Nothing matches", "Action filter", "Other contexts untouched", "Unknown context" and "Small buffers", answering the handed-out request with `putCommandAnswer2` and checking the client's reply; verify they pass

## 4. Headers and documentation

- [x] 4.1 Declare the four functions in `include/SECoP-exports.h` with Doxygen comments in SHALL's style, marked "must addition, not in SHALL", noting the filter-first argument order, the trailing `szContextID` that the SHALL-LabVIEW VIs must add, and the code range of user errors; add calls to `crates/must-ffi-ctests/c/header_check.c` and `header_check.cpp`; verify the header checks compile as C99 and C++11 and link
- [x] 4.2 Add a section on additions beyond SHALL to `COMPATIBILITY.md`: the four functions, their argument order (including the `szContextID` the filter VIs must add), filter syntax, user error code range, counting and process-wide scope, and that programs using them do not run with SHALL; mention them in `README.md` if it lists the C API

## 5. User error codes per context

Groups 2 and 4 first built process-wide, counted registrations; the user then tied them to context IDs.

- [x] 5.1 Rework the registry in `server/codes.rs`: each code records its context; no use count; a name new to the context gets a new code, the same name in the same context keeps its code with the new text, a case clash within the context or with a SECoP class gives `NAME_ALREADY_USED`; `unregister(context, code)` removes only the context's own code; `remove_context_user_errors(context)`; `to_secop(code, context, accessible)` maps only the context's codes; verify with unit tests for "Same name in two contexts", "Code of another context", "Unregister for another context" and removal by context
- [x] 5.2 Add trailing `szContextID` to `SECoP_S_registerUserError` and `SECoP_S_unregisterUserError` (`INVALID_CONTEXT_ID` for a context the library state does not hold, checked without holding both locks); remove a context's codes in `SECoP_S_doneLibrary` with `bNodeOnly` 0; pass the module's context from `ShallModule` to `to_secop`; update the declarations in `include/SECoP-exports.h` and the calls in `header_check.c`; verify with `cargo xtask check` and the header checks
- [x] 5.3 Update the C tests (`ctest_user_error_*` and `user_error_codes`) for the scenarios "Same name in two contexts", "Code of another context", "Unknown context", "Unregister for another context" and "Codes end with the context", replacing "Registered twice, unregistered once"; verify they pass
- [x] 5.4 Update the user error part of `COMPATIBILITY.md` (context argument, per-context codes, cleanup by `doneLibrary`, no counting) and the header comments; verify the documented calls match the header

## 6. Checks

- [ ] 6.1 If a LabVIEW installation is available, run the four VIs from SHALL-LabVIEW, once updated to pass `Context_ID`, against the built `SHALLserver` (32- or 64-bit) and confirm they work, including how the VI joins the filter array; otherwise record in the PR what was not tested in LabVIEW
- [x] 6.2 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686

## Workflow follow-up

- Close issue #14 with a summary of the decoded argument lists and the chosen behavior.
- Update `SECoP-filterStoredCommand.vi`, `SECoP-filterStoredCommand_2.vi`, `SECoP-registerUserError.vi` and `SECoP-unregisterUserError.vi` in SHALL-LabVIEW to pass `Context_ID` as the last argument (user, later).
- Archive this change after review; it can be archived before or after `concurrent-polled-requests`.
