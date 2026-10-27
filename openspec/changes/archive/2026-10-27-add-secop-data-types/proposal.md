# Proposal

## Why

`must` is a Rust rewrite of SHALL (SECoP hardware abstraction layer), which today is a Qt/C++ library speaking SECoP V2019-09-16 (v1.0). must replaces it without Qt, as a drop-in library for Linux and Windows (64- and 32-bit). Every later piece of `must` (SEC node server, SHALL-compatible C ABI, status TUI) needs a single, well-tested model of SECoP data: the `datainfo` descriptions and the values they describe. SHALL's VariantLib does this in ~10k lines of C++ with virtual inheritance; starting the rewrite with a clean, dependency-light Rust core gives every later stage a solid base and targets SECoP 2.0 from day one.

## What Changes

- Create the `must` Cargo workspace with a first library crate for SECoP data types (no networking, no FFI yet).
- Parse, validate and re-serialize SECoP 2.0 `datainfo` JSON for the types `double`, `scaled`, `int`, `bool`, `enum`, `string`, `blob`, `array`, `tuple`, `struct` and `command`. The new 2.0 `matrix` type is left out for now and rejected as unsupported.
- Decode JSON values against a `datainfo` into typed values, and encode typed values back to JSON, rejecting invalid input with errors that map onto SECoP's `WrongType` and `RangeError` classes.
- Validate SECoP identifiers (module, accessible and property names) per the 2.0 naming rules, and apply the case-insensitive uniqueness rule to those and to struct and enum member names.
- Support the type side of the SECoP 2.0 predefined parameter postfixes (`_min`, `_max`, `_limits`, `_enable`): recognize postfixed names, derive and check the datainfo a postfixed parameter must have, and enforce dynamic limits when validating `change`/`do` values.
- Set up CI (build, test, clippy, rustfmt, license check) for the workspace on all three targets: Linux x86_64, Windows x86_64 and Windows i686.

### Roadmap (later, separate changes; not part of this one)

1. **`add-sec-node-server`** – node/module/accessible model with properties, TCP transport, and the mandatory SECoP 2.0 messages (`*IDN?`, `describe`, `activate`/`deactivate`, `ping`, `read`, `change`, `do`, `update`, `error_*`). The optional messages `check` and `logging` are left out; a node replies to them with `ProtocolError`. Also the module-level postfix rules: a postfixed parameter needs its parent in the same module, `_limits` and `_min`/`_max` may not coexist for one parent, and `change` requests are checked against the current dynamic limits.
2. **`add-shall-c-abi`** – shared libraries with SHALL's file names (`SHALLserver64`/`32`, `SHALLvariant64`/`32` as `.dll` and `lib*.so`) exporting the same `SECoP_V_*` and `SECoP_S_*` functions (signatures, enum values, cdecl, callbacks or polling, context IDs, `QApplication*` accepted and ignored), so existing C and LabVIEW callers switch by replacing the library files. Built and tested for Linux x86_64, Windows x86_64 and Windows i686.
3. **`add-node-monitoring`** – each process hosting must nodes opens one local monitoring endpoint (all its nodes: status, connected clients, message traffic, logs) and registers it in a per-user runtime directory so tools can find every running node on the machine.
4. **`add-status-tui`** – `must-tui`, a ratatui app that discovers all monitoring endpoints and shows every running must node with its status and logs. Replaces SHALL's Qt status window: `bGUI` in `SECoP_S_initLibrary` and `SECoP_S_showStatusWindow` start it in a new terminal window (a new console on Windows, a terminal emulator on Linux when a display is available).
5. Later candidates: the `matrix` data type, the `check`/`logging` messages, WebSocket transport, UDP discovery.

Out of scope for must: SHALL's client library (`SECoP_C_*`) and its client GUI, and any Qt dependency.

## Capabilities

### New Capabilities
- `secop-identifiers`: Rules for valid SECoP names and their uniqueness within a scope.
- `secop-datainfo`: Parsing, validating and serializing `datainfo` descriptions for every SECoP 2.0 data type.
- `secop-values`: Decoding JSON values against a `datainfo`, validating them (including dynamic limits), and encoding them back to JSON, with SECoP error classification.
- `secop-parameter-postfixes`: Recognizing the predefined parameter postfixes and the datainfo rules that tie a postfixed parameter to its parent.

### Modified Capabilities
<!-- none: this is the first change in the project -->

## Impact

- New repository content: `Cargo.toml` workspace, `crates/must-types`, CI workflow, README, license files.
- License: LGPL-3.0-or-later, the same as SHALL.
- New dependencies: `serde`/`serde_json` (JSON), a base64 crate (blob transport), an ordered map crate (keep struct/enum member order as described).
- No public network or C API yet; those arrive in the roadmap changes above. The Rust API of `must-types` is the foundation they will build on.
