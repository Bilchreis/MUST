# Design

## Context

See proposal.md for the motivation and the specs for the required behavior.

**Today's code.**
- `protocol.rs` parses `check` into `Request::Unsupported`. `runtime/connection.rs` answers it with `ProtocolError`.
- The model already accepts a boolean `checkable` on parameters and commands (`model.rs`) and publishes it unchanged.
- `change` validation (request mode, dynamic limits from `_min`/`_max`/`_limits`) and the `do` argument validation live in `runtime/connection.rs` and run before the handler is called.
- `ShallModule` (`must-ffi/src/server/handler.rs`) serves one module of the C ABI:
  - **Callback mode:** calls the getter, setter or command function on the blocking pool, behind a per-module gate.
  - **Polling mode:** queues a `state::Pending` with an action code and waits for `putCommandAnswer`, with the polled request timeout.
  - `state.rs` and `filter.rs` (from `user-errors-and-filtered-commands`) restrict the action filter to 0–3.

## Goals / Non-Goals

**Goals:**
- `check` reuses the validation and the request path of `change`/`do`, so that a check never accepts a value that the matching `change` would reject before reaching the host.
- Hosts that never publish `checkable: true` see no change: no action 4, no new calls.

**Non-Goals:**
- Returning a value other than the checked one in `checked` (SECoP sends the verified value back). A host cannot suggest a "closest valid" value. SECoP's example puts that in the error report's qualifiers, which the C ABI has no way to fill.
- Enforcing SECoP's rule that a check must not depend on the module's status. That is the host's responsibility, and it is documented.

## Decisions

### One code path for checks
`Request::Check(spec, data)` resolves the accessible, then:
- **Parameter:** reuses the `change` validation (readonly → `ReadOnly`, request mode, dynamic limits).
- **Command:** reuses the `do` argument validation.

Only after that does it look at `checkable` and call `handler.check(ctx, accessible, value)`. The reply is built by the same reply writer, with action `checked`/`error_check`. It does not touch `Hub` state: no cache write, no update fan-out, no ordering barrier.

Order of checks: unknown accessible → `NoSuchParameter`; readonly → `ReadOnly`; not checkable → `NotCheckable`; value → `WrongType`/`RangeError`; then the handler. A client learns about an accessible that will never be checkable before it learns about its value.

### `ModuleHandler::check` returns `Result<(), SecopError>`
The default is `NotImplemented` "check is not implemented for <accessible>". The node sends the validated value back itself, so handlers cannot alter it.
- *Alternative:* return a `Reading`, like `change`. Rejected: SECoP's `checked` echoes the request. A modified value would read as "this is what would be set", which the spec does not define.

### C callback: `SECoP_S_checkFunction(const char* name, const CSECoPbaseType* pValue, enum SECoP_S_error* piError)`
The shape follows `SECoP_S_callFunction` without result and timestamp. `pValue` is NULL for a command without argument. `SECoP_S_setCheckFunction(ptr, szContextID)` acts on the focus, like the `addProperty*` calls. It stores the function on the build item and sets `checkable: true`. A NULL pointer clears both.
- *Alternative:* a new `SECoP_S_add*` variant that registers a parameter with getter, setter and checker in one go. Rejected: it doubles the number of add functions, and commands would need one too.

### Polling: action 4 on the existing queue
`state::Pending` already carries an action code. Checks are enqueued with `action::CHECK = 4` and the validated value, exactly like `change`. The filter's action range becomes 0–4. `putCommandAnswer[2]` ignores the value for action 4. A non-negative code gives `checked`, and a negative code goes through `to_secop` like any answer, including registered user errors.

### Validation at `nodeComplete`
In callback mode, an accessible with `checkable: true` and no check function gets `MISSING_MANDATORY`, so a node never advertises checks it cannot answer. In polling mode the property alone is the opt-in, and there is nothing to check at build time.

## Risks / Trade-offs

- [A host already publishes `checkable: true` (accepted and passed through today) and would get unexpected action 4 in its polling loop] → Unlikely, since checks have only ever returned `ProtocolError`. `COMPATIBILITY.md` calls it out. A VI that does not know action 4 leaves it unanswered, and the client gets `TimeoutError`, not a crash.
- [SHALL-built hosts in callback mode that set `checkable: true` stop at `nodeComplete`] → Intended: the alternative is a node that lies about its features. The error names the accessible.
- [The archive order with `add-logging-messages` and `user-errors-and-filtered-commands`] → Recorded in the proposal's impact and in the workflow follow-up of tasks.md.
