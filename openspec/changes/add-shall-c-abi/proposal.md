# Proposal

## Why

Many SHALL users do not write C++ against SHALL; they call its C functions from LabVIEW or plain C, often 32-bit LabVIEW on Windows. For them must is only useful if it is a drop-in replacement: same library files, same exported functions, same behavior, but without Qt. With `must-types` and `must-server` in place, this change adds that C ABI.

## What Changes

- Build four shared libraries with SHALL's names: `SHALLvariant64`/`SHALLvariant32` and `SHALLserver64`/`SHALLserver32`, as `.dll` (+ import `.lib`) on Windows x86_64/i686 and `lib*.so` on Linux x86_64.
- Export every C function of SHALL's VariantLib (`SECoP_V_*`) and ServerLib (`SECoP_S_*`) with identical names, parameter lists, enum values, buffer conventions and cdecl calling convention, so callers relink or simply swap the library files.
- Reproduce SHALL's server behaviors: building nodes by "focus", callback mode and polling mode (`SECoP_S_getStoredCommand[2]`/`SECoP_S_putCommandAnswer[2]`), context IDs, pushed updates, error and warning codes, the 60 s timeout for unanswered polled requests.
- Accept and ignore SHALL's Qt-related arguments (`QApplication*`, `SECoP_S_setManyThreads`). `bGUI` and `SECoP_S_showStatusWindow` are accepted; they start the status TUI once `add-status-tui` lands and do nothing until then.
- Ship Qt-free versions of SHALL's public headers (`SECoP.h`, `SECoP-defines.h`, `SECoP-types.h`, `SECoP-exports.h`, `SECoP-Variant.h`).
- **BREAKING** (compared to SHALL): the wire protocol is SECoP 2.0 instead of V2019-09-16; the C++-only functions `SECoP_V_printString` (returns `QByteArray`) and `SECoP_V_printStream` (takes `QTextStream&`) and the C++ class interface of `CSECoPbaseType` are not provided — `CSECoPbaseType` is an opaque handle.

## Capabilities

### New Capabilities
- `shall-variant-abi`: The `SECoP_V_*` C functions for creating, inspecting, modifying, converting and freeing SECoP values through opaque `CSECoPbaseType` handles.
- `shall-server-abi`: The `SECoP_S_*` C functions for building and running SEC nodes from C/LabVIEW, including callbacks, polling, context IDs and error codes.
- `shall-abi-packaging`: Library file names, headers, platforms, calling convention and dependency guarantees of the drop-in libraries.

### Modified Capabilities
<!-- none -->

## Impact

- New crates: `crates/must-ffi` (the `SHALLserver` cdylib holding all logic) and `crates/must-ffi-variant` (the thin `SHALLvariant` cdylib), plus an `xtask` for building and renaming the libraries per target.
- New C header directory `include/` (derived from SHALL's headers, LGPL-3.0-or-later).
- CI builds and runs C conformance tests on Linux x86_64, Windows x86_64 and Windows i686.
- Depends on `add-sec-node-server` (runtime and model) and `add-secop-data-types`.
