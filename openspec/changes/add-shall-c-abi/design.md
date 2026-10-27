# Design

## Context

SHALL ships two libraries per bitness: `SHALLvariant{32,64}` (`SECoP_V_*`, the value object `CSECoPbaseType`) and `SHALLserver{32,64}` (`SECoP_S_*`), which links against the variant library. Callers (C, LabVIEW "Call Library Function" nodes) refer to these file names and C symbol names. In SHALL every call is marshalled onto a Qt main thread (`QMetaObject::invokeMethod(..., BlockingQueuedConnection)`); building uses a per-context "focus" on the last created item; polled requests time out after `SECOP_POLLING_TIMEOUT` (60 s) and are answered with `SECoP_S_ERROR_TIMEOUT`. `must-server` (change `add-sec-node-server`) provides the node model, an async handler trait and a library-owned runtime; `must-types` provides datainfo and values.

## Goals / Non-Goals

**Goals:**
- Binary drop-in: same file names, symbols, signatures, enum values and observable behavior for C callers.
- One copy of all state per process, even though callers load two libraries.
- Conformance proven by C tests on all three targets.

**Non-Goals:**
- SHALL's C++ class API (`CSECoPbaseType` methods, `SECoP_dataPtr`), `SECoP_V_printString`, `SECoP_V_printStream`.
- SHALL's client library (`SECoP_C_*`).
- Bit-identical log or error-string texts.

## Decisions

### Two cdylibs, one implementation
`crates/must-ffi` builds `SHALLserver{32,64}` and contains everything: the handle table, all `SECoP_V_*` and `SECoP_S_*` implementations. `crates/must-ffi-variant` builds `SHALLvariant{32,64}`, a thin cdylib whose `SECoP_V_*` exports forward to the same functions imported from `SHALLserver` (`#[link(name = "SHALLserver64")]` `extern "C"` declarations). So handles always live in one heap and one table, whichever library the caller used.
*Alternative:* each library embeds `must-types` and exchanges values as JSON. Rejected: two heaps on Windows make freeing a handle from the "other" DLL undefined, and every crossing would serialize.
*Alternative:* Windows `.def` export forwarding. Rejected: Linux has no equivalent, and one mechanism for both is simpler to test.

### Handles
`CSECoPbaseType*` points to a heap-allocated `Handle { info: Arc<DataInfo>, value: Option<Value>, extra: Map }` (`Box::into_raw`). SHALL's handles carry type and value together, so do ours; `SECoP_V_create` makes a handle with a default value, `SECoP_V_fromJSON` with a parsed one. SHALL's 1.0-era descriptions are accepted by `must-types` as they are a subset of 2.0 datainfo. Every entry point checks for NULL and catches panics (`catch_unwind`), returning the function's documented failure value. Strings returned as `const char*` (enum names, struct names, `getErrorString`) point into the handle or to static data, matching SHALL's lifetime rules.

### Positional access
`uPosition` addresses a pre-order walk of the value: position 0 is the root; containers (struct, tuple, generic array) are one position followed by their items; arrays of simple types (`SECoP_VT_ARRAY_DOUBLE`, …) are a single position with `uCount` elements accessed by `uIndex`. The walk is computed on demand from `DataInfo`; no cached flat table, since handles are mutable through `modify*`.

### Context, focus and threads
A global `Mutex<State>` maps context IDs to `{ nodes, focus cursor, stored-request queue }`. `SECoP_S_initLibrary` registers a context (reference-counted per ID) and creates the shared tokio runtime on first use; `SECoP_S_doneLibrary(ctx)` stops that context's nodes and drops it; when no context remains the runtime is shut down. Functions without a context parameter (`SECoP_S_deleteNode`, `SECoP_S_setAddFocus`, `SECoP_S_updateParameter`, `SECoP_S_putCommandAnswer`) find their target by node name or request id, as in SHALL. No Qt-style main thread: all functions are callable from any thread and only hold the global lock briefly.

### Handlers for callbacks and polling
One `ModuleHandler` implementation per module:
- Callback mode: run the C getter/setter/command function on tokio's blocking pool (`spawn_blocking`), so a slow LabVIEW callback never stalls the runtime; convert `CSECoPbaseType**` in/out through handles.
- Polling mode: push `StoredRequest { id, action, name, value, created }` into the context's queue and await a oneshot; `SECoP_S_getStoredCommand[2]` pops, `SECoP_S_putCommandAnswer[2]` completes. A sweeper fails requests older than 60 s with `TimeoutError` (`SECoP_S_ERROR_TIMEOUT` toward the application).
Module property `pollinterval` (SHALL's polling mechanism) maps to `must-server` polling.

### Build model to `must-server`
The focus API drives `must-server` builders. Unknown non-underscore properties are added as non-standard properties (allowed explicitly by `sec-node-model`) and return `SECoP_S_WARNING_CUSTOM_PROPERTY`. `SECoP_S_nodeComplete` maps `ModelError`s to SHALL codes (missing mandatory → `SECoP_S_ERROR_MISSING_MANDATORY`, bad name → `SECoP_S_ERROR_INVALID_NAME`, duplicate → `SECoP_S_ERROR_NAME_ALREADY_USED`, …) and keeps the full list for `SECoP_S_showErrors`, which logs it and prints to stderr.

### Error code mapping toward clients

| `SECoP_S_error` from application | SECoP 2.0 class |
|---|---|
| `INVALID_MODULE` | `NoSuchModule` |
| `INVALID_PARAMETER` | `NoSuchParameter` |
| `INVALID_COMMAND` | `NoSuchCommand` |
| `READONLY` | `ReadOnly` |
| `INVALID_VALUE` | `WrongType` |
| `OUT_OF_RANGE` | `RangeError` |
| `NOT_IMPLEMENTED` | `NotImplemented` |
| `TIMEOUT` | `TimeoutError` |
| `COMM_FAILED` | `CommunicationFailed` |
| `COMMAND_FAILED` | `HardwareError` |
| `COMMAND_RUNNING` | `CommandRunning` |
| `IS_BUSY` | `IsBusy` |
| `IS_ERROR` | `IsError` |
| `DISABLED` | `Disabled` |
| `SYNTAX` | `ProtocolError` |
| `INTERNAL` and all others | `InternalError` |

`COMMAND_FAILED` was SECoP 1.0's `CommandFailed`, which 2.0 no longer has; `HardwareError` is the closest 2.0 class.

### Status display hook
`bGUI` and `SECoP_S_showStatusWindow` call a `StatusDisplay` hook that is a no-op in this change; `add-status-tui` implements it.

### Packaging and names
Cargo cannot name a library differently per target, so an `xtask` (`cargo xtask dist --target <triple>`) builds the cdylibs and copies/renames them to `dist/<triple>/` with the 32/64 suffix, sets the Linux SONAME via `-C link-arg=-Wl,-soname,...`, and copies the headers. Windows builds link the CRT statically (`+crt-static`). Symbol names are checked after the build (`nm -D` / `dumpbin /exports`) against a list extracted from SHALL's headers.

### Headers
`include/` holds SHALL's five public headers with Qt removed: `SHALL_EXPORT` falls back to `__declspec(dllimport)` / nothing, `QApplication` is always `typedef void QApplication`, and `CSECoPbaseType` is an opaque struct for C and C++. Function declarations stay byte-identical where possible so existing code compiles unchanged.

### Conformance tests
`tests/c/` contains C programs (built with the `cc` crate from a test harness crate) for: the `SECoP.h` polling example, a callback-mode node, context ID scenarios ported from SHALL's `tst_contextid.cpp`, variant round trips and positional access. A Rust test client drives the nodes over TCP. The same tests run on all three CI targets; the i686 job uses the 32-bit MSVC toolchain.

## Risks / Trade-offs

- [SHALL behaviors that are not documented in headers may differ] → Port SHALL's own tests and derive expected values from SHALL's sources; record intentional differences in a `COMPATIBILITY.md`.
- [LabVIEW cannot be run in CI] → Runtime symbol lookup test (`LoadLibrary`/`dlopen` + `GetProcAddress`/`dlsym`) mimics how LabVIEW binds; plus a documented manual smoke test with a LabVIEW VI.
- [Clients that rely on SECoP 1.0 wire details break] → Documented as breaking; SECoP 2.0 clients are expected to also support 1.0 nodes, not the other way round.
- [Forwarding DLL requires both files to be deployed together] → SHALL also required both; documented in the README.

## Migration Plan

Users replace `SHALLserver{32,64}` and `SHALLvariant{32,64}` (and remove the Qt DLLs) with the must builds; recompiling is only needed for C++ code that used the class API. Rollback is putting the SHALL files back.
