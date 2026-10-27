# Spec Delta

## Purpose

Defines how the drop-in libraries are delivered so that SHALL users can swap libraries without code changes: file names, platforms, headers, calling convention and dependencies.

## ADDED Requirements

### Requirement: Library file names
The build SHALL produce `SHALLserver64` and `SHALLvariant64` for 64-bit targets and `SHALLserver32` and `SHALLvariant32` for 32-bit targets, as `<name>.dll` plus import library `<name>.lib` on Windows and `lib<name>.so` (with a matching SONAME) on Linux.

#### Scenario: Windows 32-bit artifacts
- **WHEN** the libraries are built for `i686-pc-windows-msvc`
- **THEN** the output contains `SHALLserver32.dll`, `SHALLserver32.lib`, `SHALLvariant32.dll` and `SHALLvariant32.lib`

#### Scenario: Linux artifacts
- **WHEN** the libraries are built for `x86_64-unknown-linux-gnu`
- **THEN** the output contains `libSHALLserver64.so` and `libSHALLvariant64.so` with SONAMEs equal to their file names

### Requirement: Supported platforms
The libraries SHALL be built and their conformance tests SHALL pass for Linux x86_64, Windows x86_64 and Windows i686.

#### Scenario: CI matrix
- **WHEN** CI runs for a commit
- **THEN** the C conformance tests run against the built libraries on all three targets

### Requirement: C calling convention and plain symbol names
Every exported function SHALL use the C (cdecl) calling convention and be exported under its plain C name without decoration or mangling.

#### Scenario: Name lookup by LabVIEW-style loading
- **WHEN** a program loads `SHALLserver32.dll` at runtime and looks up `SECoP_S_initLibrary` by name
- **THEN** the lookup succeeds and the call works with cdecl

### Requirement: Variant functions in both libraries' set
`SHALLvariant` SHALL export the `SECoP_V_*` functions and handles created through it SHALL be usable with `SHALLserver` functions in the same process and vice versa.

#### Scenario: Handle across libraries
- **WHEN** a handle is created with `SECoP_V_createSimpleDouble` from `SHALLvariant64` and passed to `SECoP_S_updateParameter` from `SHALLserver64`
- **THEN** the update is sent and the handle can be destroyed with `SECoP_V_destroy`

### Requirement: Qt-free headers
The distribution SHALL include SHALL's public headers (`SECoP.h`, `SECoP-defines.h`, `SECoP-types.h`, `SECoP-exports.h`, `SECoP-Variant.h`) with the same declarations for the exported functions and types, compiling as C99 and C++11 without Qt and with `CSECoPbaseType` as an opaque type.

#### Scenario: Compile SHALL example code
- **WHEN** a C file containing the example from SHALL's `SECoP.h` is compiled as C99 against the headers and linked with the libraries
- **THEN** it compiles without Qt include paths and runs

### Requirement: No Qt or system runtime dependencies
The libraries SHALL not depend on Qt and SHALL only link against libraries present on a standard installation of the target OS (plus the MSVC runtime statically linked on Windows).

#### Scenario: Dependency check
- **WHEN** the dependencies of the built libraries are listed (`ldd` / `dumpbin /dependents`)
- **THEN** no Qt library and no non-system library other than `SHALLserver` (for `SHALLvariant`) appears
