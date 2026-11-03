# Proposal

## Why

SECoP 2.0 defines `check`, a dry run of `change` and `do`. A client asks whether a value would be accepted without changing anything, for example to validate a measurement plan before running it. must answers every `check` with a `ProtocolError` today. The `checkable` accessible property already passes through to the description, so a node can claim to support checks that it then refuses. Check was deferred when must started. The spec wants checks to go "beyond a simple validity check based on the accessible's datainfo", so the application (the host) has to make the final decision.

## What Changes

- **must-server answers `check <module>:<accessible> [<value>]`**:
  1. Resolve the accessible. An accessible that is not `checkable: true` gets `NotCheckable`.
  2. Validate the value in request mode, against the parameter's datainfo with its dynamic limits, or against the command's argument. A failure is answered with `WrongType`/`RangeError` without asking the handler.
  3. Ask the module's handler.
  4. Reply `checked <module>:<accessible> [<value>, {"t": ...}]`, or `error_check` with the handler's error.

  The reply goes only to the requesting connection. The value cache, updates and polling are not touched. A readonly parameter gets `ReadOnly`.
- **Rust handlers** get `ModuleHandler::check(ctx, accessible, value)`. The default returns `NotImplemented`, so a module that declares `checkable` but does not implement it answers honestly.
- **C ABI, must additions (not in SHALL)**:
  - **Callback mode:** new callback type `SECoP_S_checkFunction(name, pValue, piError)` and new export `SECoP_S_setCheckFunction(ptrToCheck, szContextID)` for the focused parameter or command. Setting it makes the accessible checkable. An accessible with `checkable: true` but no check function makes `SECoP_S_nodeComplete` fail with `SECoP_S_ERROR_MISSING_MANDATORY`.
  - **Polling mode:** new action value `SECoP_S_ACTION_CHECK = 4`. A check of a checkable accessible is queued like `change` and `do`, handed out by `SECoP_S_getStoredCommand[2]` and `SECoP_S_filterStoredCommand[2]`, and answered with `SECoP_S_putCommandAnswer[2]`, whose value is ignored. Only hosts that publish `checkable: true` ever see action 4.
  - The answer's code maps as usual: `OUT_OF_RANGE` → `RangeError`, `INVALID_VALUE` → `WrongType`. For SECoP's `Impossible`, a host registers it with `SECoP_S_registerUserError("Impossible", …)`.
- **Headers and docs**: `SECoP-exports.h` and `SECoP-types.h` declare the additions. `COMPATIBILITY.md` lists them.

No breaking change. Accessibles without `checkable: true` answer `check` with `NotCheckable` rather than `ProtocolError`. SECoP prescribes this for a node that implements `check`.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `sec-node-protocol`: "Optional and custom messages" is replaced by "Empty lines" and "Logging not supported", since its scenario "Check not supported" no longer holds. New requirements: "Check" and "Check replies".
- `sec-node-runtime`: "Module handlers" also routes `check`.
- `shall-server-abi`: new requirements "Checks in callback mode", "Checks in polling mode" and "Check functions beyond SHALL".

## Impact

- `crates/must-server`:
  - `protocol.rs`: parse `check`.
  - `runtime/connection.rs`: answer it; drop the unsupported branch for `check`.
  - `handler.rs`: `ModuleHandler::check`.
  - `model.rs`: `checkable` on readonly parameters.
- `crates/must-ffi`:
  - `server/handler.rs`: check via callback or queue.
  - `server/api.rs`: `SECoP_S_setCheckFunction`; `nodeComplete` checks.
  - `server/state.rs`: action 4 in queue and filter.
  - `server/build.rs`: per-accessible check function.
- `include/SECoP-exports.h`, `include/SECoP-types.h`: declarations.
- `crates/must-ffi-ctests`: header checks and C tests.
- Docs: `COMPATIBILITY.md`, `must-server` crate docs.
- SHALL-LabVIEW (outside this repository): VIs that dispatch on the action need a case for 4 before a LabVIEW node can publish `checkable: true`.
- Related:
  - Archive this change before `add-logging-messages`, which removes the "Logging not supported" requirement added here.
  - `user-errors-and-filtered-commands` must be archived first, because the filter functions gain action 4.
