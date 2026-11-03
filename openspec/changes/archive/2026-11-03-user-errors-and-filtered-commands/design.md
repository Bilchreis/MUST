# Design

## Context

See proposal.md for the motivation and specs for the required behavior.

**Where the argument lists come from.** The SHALL-LabVIEW 2019 VIs (`~/Nextcloud/shall-labview/shall-labview-2019-v0.1.zip`, the same VIs as the GitHub repo) contain each Call Library Function Node as a cluster type labeled with the function name in the VI's type table (`VCTP`). Its members are the return type and then the parameters, in call order, with their names. Decoding `SECoP_S_getStoredCommand`, `SECoP_S_getStoredCommand2` and `SECoP_S_putCommandAnswer2` this way gives exactly SHALL's declarations, which confirms the method. For the four missing functions it gives:

| Function | Parameters, in order |
|---|---|
| `filterStoredCommand` | `szFilterAccessibles` (C string), `iFilterAction` (i32), pointer, pointer, `szAccessible` (C string), pointer, pointer |
| `filterStoredCommand2` | `szFilterAccessibles`, `iFilterAction`, pointer, pointer, `szAccessible`, pointer, `szValue` (C string), pointer |
| `registerUserError` | `szSECoPerror` (C string), `szMessageText` (C string) |
| `unregisterUserError` | `iError` (i32) |

The diagram's terminal names fill in the unnamed pointers: `pllId`, `piAction`, `piAccessibleSize`, `pValue`/`piValueSize`, in the same positions as in `getStoredCommand[2]`. The VIs' help texts give their meaning:
- **filter:** "check, if there is something to do for selected accessibles only"; "0 = any action".
- **register:** "register your own SECoP errors with own text description. This function will return an unused SECoP-error.ctl code, which you should use with putCommandAnswer". The name starts with a letter or underscore and uses letters, digits and underscores only.

The call nodes point to `SECoP-LabVIEW2015/32bit/SECoP32.dll`, SHALL's predecessor. No SHALL DLL ever exported the functions.

**Today's code.**
- Polled requests sit in a `VecDeque` per context (`server/state.rs`). Ids are global and increasing. `next_request` (`server/api.rs`) pops the front of one context's queue and moves the request to `handed_out`.
- `codes.rs` maps codes to descriptions and error classes with fixed `match`es.
- `SecopError::class` is a closed, `Copy` enum (`must-types/src/error.rs`). Only `report::error_report` turns it into the class name sent on the wire.

## Goals / Non-Goals

**Goals:**
- The SHALL-LabVIEW VIs work against must once they pass `Context_ID` as one more argument at the end of each of the four calls.
- Plain C programs can use the functions too, with declarations in `SECoP-exports.h`.

**Non-Goals:**
- Changing the SHALL-LabVIEW VIs in this change. The user updates the four VIs later. Whether LabVIEW can show a code outside the values of `SECoP-error.ctl` is up to the VIs.
- Custom error classes in `SECoP_S_updateParameter`. It has no error argument.

## Decisions

### Argument order from the VIs, with `szContextID` appended
All four functions take the VIs' arguments in the VIs' order and then `szContextID`, last as in every other `SECoP_S_*` function that takes one. For the filter functions the filter arguments come first.
- **Why not the VIs' list alone:** without a context the functions would have to search every context's queue, and an empty filter could take requests of another context.
- **Why append rather than reorder:** the VI fix is one more parameter on each call node, wired to the `Context_ID` from `SECoP-Initialize.vi` that the other VIs already use. The header comment points out the order.

*Alternative:* `getStoredCommand[2]`'s order followed by the filter and the context. Rejected by the user: every parameter of the VI call nodes would have to be rewired.

### Filter within one context
`szContextID` is read like in `getStoredCommand[2]`: NULL is the default context `""`, and an unknown context gives `INVALID_CONTEXT_ID`.
- **Implementation:** `take_matching(context, filter)` in `state.rs` takes the oldest matching request (the lowest id) from that context's queue, and moves it to `handed_out` as `next_request` does.
- **Cost:** a linear scan of the queue, which holds at most a few hundred requests (64 per module).

*Alternative:* searching all contexts, which the VIs' original argument list would have needed. Rejected: one context could take another's requests.

### Filter format: prefixes on `:` boundaries, liberal separators
An entry `node`, `node:module` or `node:module:accessible` matches when its parts equal the first parts of the request's `node:module:accessible` name, ignoring ASCII case. Entries are split at `,`, `;` and ASCII whitespace.
- **Why liberal separators:** the VI's input is a string array, but the binary does not show how the VI joins it into one string. The usual LabVIEW ways (Array To Spreadsheet String: tab separators and a trailing line end; a joined comma list) all parse.
- **Why prefixes:** a host loop can serve a whole module with one entry.
- **No `module:accessible` without the node:** two-part entries would be ambiguous.
- **Checks:** every part is checked with `must_types::validate_identifier`. A filter with a bad entry is refused as a whole (`INVALID_NAME`), so a typo does not make a loop quietly take nothing.

*Alternatives:* exact names only (rejected: a module-level loop is a natural use); glob patterns (rejected: more syntax with no use case).

### Custom class name on `SecopError`
`SecopError` gets `custom_class: Option<Box<str>>` and a constructor `SecopError::custom(name, message)`. `error_report` sends `custom_class` when set, otherwise `class.as_str()`. A boxed `str` (16 bytes) rather than a `String` (24) keeps `SecopError` under clippy's 128-byte `result_large_err` limit; its extra map is an `IndexMap` because of serde_json's `preserve_order`.
- **`class` of a custom error:** `InternalError`, the class unknown codes get today, so code that inspects `class` keeps working.
- **A standard name** (e.g. `HardwareError`) gives that class with no override.
- **Checks:** `custom` validates the name as a SECoP identifier and panics in debug builds on an invalid one. The FFI validates before registering.

*Alternative:* an `ErrorClass::Other(String)` variant. Rejected: `ErrorClass` is `Copy` and matched on in many places, and `FromStr` would have to change meaning.

### Registry of user error codes in `codes.rs`, per context
The registry is a process-wide `Mutex` with:
- `by_code: HashMap<c_int, UserError { context, name, text, description }>`,
- `next: c_int`, starting at -1000 and decreasing,
- `descriptions`: every `name: text` string handed out, kept for the process lifetime.

How the functions use it:
- **Register:** the context must exist in the library state (created by `SECoP_S_initLibrary` or `SECoP_S_createNode[2]`), else `INVALID_CONTEXT_ID`. A name new to the context gets `next`, and `next` decreases. A name the context already has (exact spelling) keeps its code and gets the new text. A name equal to one of the context's names, or to a SECoP 2.0 class name, only after lowercasing gives `NAME_ALREADY_USED`. Other contexts' names do not matter: two contexts registering `_Overheated` get two codes.
- **Unregister:** removes the code if the given context registered it, else `INVALID_VALUE` (also for SHALL's own codes).
- **Lookups:** `to_secop(code, context, accessible)` maps a code only if `context` registered it, else it treats the code as unknown (`InternalError`). It builds `SecopError::custom(name, "{accessible}: {text}")`, the same text layout as SHALL's codes. `ShallModule` passes its node's context. `description` (for `SECoP_S_getErrorString`, which has no context) describes any registered code; codes are unique in the process.
- **Cleanup:** `SECoP_S_doneLibrary` with `bNodeOnly` 0 removes the context's codes. With `bNodeOnly` 1 they stay, and the context gets them back when it is initialized again.
- **`SECoP_S_getErrorString`:** returns a pointer into `descriptions`. Each `name: text` combination is created once and never freed, so the pointer stays valid as SHALL's static strings do. The memory grows only with distinct registrations.

Why each choice:
- **-1000 and below:** leaves room if SHALL adds codes after -30. A failure code from `registerUserError` (-2, -18, -30) is easy to tell apart from a registered code.
- **Never reused:** a late answer with an old code cannot turn into another error.
- **Per context, decided by the user:** a context only uses and removes its own codes, so one LabVIEW program's `SECoP-Exit.vi` cannot remove an error another program still uses, without a use count.
- **Idempotent by name within a context:** a VI run twice in one LabVIEW session gets the same code.
- **Own mutex, never held together with the state lock:** the context check takes the state lock and releases it before the registry is locked, and `doneLibrary` clears the registry after releasing the state lock.

*Alternatives:* codes from -31 down (rejected: they would collide with codes SHALL might add); one process-wide code per name with a use count (the first version of this change; replaced when the functions got a context ID); any registered code usable by every context (rejected by the user: a context would depend on another's registrations).

## Risks / Trade-offs

- [The join used by `SECoP-filterStoredCommand.vi` is not confirmed] → The parser accepts all common delimiters. A task checks the VI in LabVIEW if one is available. Otherwise COMPATIBILITY.md documents the accepted syntax.
- [`SECoP-error.ctl` in LabVIEW may not show codes ≤ -1000 by name] → This does not affect behavior: the VI passes the number through to `putCommandAnswer`. It is documented.
- [Programs that use these functions do not run with SHALL] → They do not run with SHALL today either. The header and COMPATIBILITY.md mark the functions as must additions.
- [One of the four VIs that is not yet updated passes no `szContextID`, so the library reads an undefined value from the stack in its place, which can crash] → The VIs fail with every library today anyway. The header and `COMPATIBILITY.md` state the extra argument. Task 5.1 checks the updated VIs.

## Migration Plan

Additive, nothing to migrate. Rollback removes the four exports.
