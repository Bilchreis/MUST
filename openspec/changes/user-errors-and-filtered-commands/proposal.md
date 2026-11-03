# Proposal

## Why

The VIs of SHALL-LabVIEW call four library functions that neither SHALL nor must exports (issue #14): `SECoP_S_filterStoredCommand`, `SECoP_S_filterStoredCommand2`, `SECoP_S_registerUserError` and `SECoP_S_unregisterUserError`. A VI that reaches one of these calls fails, with SHALL and with must. No released library ever had them, so the VIs fix only their argument lists. must is free to decide what the functions do. Polling-mode requests of one module can now be handed out at once (`concurrent-polled-requests`), so several host loops that each serve some of the accessibles can run in parallel. The filter functions are what those loops need.

## What Changes

- **New: `SECoP_S_filterStoredCommand` and `SECoP_S_filterStoredCommand2`.** They work like `SECoP_S_getStoredCommand[2]` but hand out only the oldest queued request that matches a list of accessibles and an action (`0` = any action). The argument order is the one the SHALL-LabVIEW VIs pass, with the two filter arguments first. must adds `szContextID` as the last argument, as in every other `SECoP_S_*` function that takes one, and searches only that context's queue. The VIs pass no context ID today and will be updated to pass it.
- **New: `SECoP_S_registerUserError` and `SECoP_S_unregisterUserError`.**
  - Registering takes a SECoP error class name, a text and a context ID, and returns a new error code of that context, outside SHALL's range of codes.
  - When the application answers a request of that context with that code (from a callback or through `SECoP_S_putCommandAnswer[2]`), the client receives an error report with that class name and text.
  - `SECoP_S_getErrorString` describes registered codes.
  - Unregistering removes a code, and so does a full `SECoP_S_doneLibrary` of its context. A code is never reused.
  - As with the filter functions, must adds `szContextID` as the last argument; the VIs will be updated to pass it.
- **Custom error classes in error reports.** must-types and must-server can send an error class outside SECoP 2.0's list. SECoP allows this and tells clients to treat such classes as generic errors. Rust handlers can use custom classes too.
- **Headers and docs.** `SECoP-exports.h` declares the four functions, marked as must additions. `COMPATIBILITY.md` lists them as additions that SHALL lacks.

No breaking change: every existing function and code behaves as before. The four VIs of SHALL-LabVIEW need one more argument, but they do not work with any library today.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `shall-server-abi`: new requirements "Filtered polled requests", "User error codes" and "Functions beyond SHALL"; "Error code mapping" covers registered codes.
- `sec-node-protocol`: "Data and error reports" allows custom error class names.

## Impact

- `crates/must-types`: `SecopError` can carry a custom class name, and `error_report` sends it.
- `crates/must-ffi`:
  - `server/codes.rs`: registry of user error codes per context; `description` and `to_secop` consult it.
  - `server/api.rs`: the four new exports; `SECoP_S_doneLibrary` removes a context's codes.
  - `server/handler.rs`: maps answers with the module's context.
  - `server/state.rs`: taking a matching request from a context's queue.
- SHALL-LabVIEW (outside this repository): `SECoP-filterStoredCommand.vi`, `SECoP-filterStoredCommand_2.vi`, `SECoP-registerUserError.vi` and `SECoP-unregisterUserError.vi` must pass `Context_ID` as an extra last argument before they work with must.
- `include/SECoP-exports.h`: declarations of the four functions.
- `crates/must-ffi-ctests`: header check and C tests for both features.
- Docs: `COMPATIBILITY.md`; the `must-server` crate docs for custom error classes.
- Related: `concurrent-polled-requests`, implemented but not archived, modifies "Polling mode" in `shall-server-abi`. This change only adds requirements next to it, so the two can be archived in either order.
