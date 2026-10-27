# Tasks

## 1. Build and packaging skeleton

- [ ] 1.1 Add `crates/must-ffi` (cdylib) and `crates/must-ffi-variant` (cdylib forwarding to `SHALLserver`) with one exported dummy function each; verify both build on all three CI targets
- [ ] 1.2 Add `xtask dist` that builds per target, renames to `SHALLserver{32,64}`/`SHALLvariant{32,64}` (`.dll` + `.lib`, `lib*.so` with SONAME), links the Windows CRT statically and copies headers into `dist/<triple>/`; verify the file lists from the library-name scenarios in specs/shall-abi-packaging
- [ ] 1.3 Add `include/` with SHALL's five headers made Qt-free and `CSECoPbaseType` opaque; verify a C99 and a C++11 file including `SECoP.h` compile with gcc and MSVC (x64 and x86) without Qt
- [ ] 1.4 Add a symbol check script comparing exported names (`nm -D` / `dumpbin /exports`) with the function list extracted from the headers, and a dependency check (`ldd` / `dumpbin /dependents`); verify both run in CI and fail on a missing symbol or a Qt dependency

## 2. Variant ABI

- [ ] 2.1 Implement the handle type, NULL and panic guards, `SECoP_V_create*`, `SECoP_V_fromJSON` (with hint), `SECoP_V_toJSON`, `SECoP_V_getType`, `SECoP_V_copy`, `SECoP_V_destroy` and `SECoP_V_createSECoPStatus`; verify C tests for creation, JSON round trips, buffer sizes and NULL handling pass
- [ ] 2.2 Implement the positional walk with `SECoP_V_getInfo`/`getInfo2`, `SECoP_V_get*`, `SECoP_V_modify*`, enum, struct-name and additional-property accessors; verify C tests for the positional scenarios in specs/shall-variant-abi, including nested structs and simple-type arrays
- [ ] 2.3 Implement `SECoP_V_compare` and `SECoP_V_printFile`; verify C tests for all four comparison results and printed output of a struct
- [ ] 2.4 Implement the forwarding exports in `must-ffi-variant`; verify the cross-library handle scenario passes

## 3. Server ABI: building nodes

- [ ] 3.1 Implement global state with context IDs, `SECoP_S_initLibrary`/`doneLibrary`/`setManyThreads`/`getErrorString`, and the shared runtime lifecycle; verify the initialization and done-for-one-context scenarios as C tests
- [ ] 3.2 Implement focus-based building (`createNode[2]`, `deleteNode`, `addModule`, `addCommand`, `add*Parameter`, `addProperty*`, `setAddFocus`) with warning codes, and `nodeComplete`/`showErrors` with error mapping; verify the building, two-contexts, warnings and validation-failure scenarios pass

## 4. Server ABI: runtime

- [ ] 4.1 Implement the callback-mode handler on the blocking pool, including `pollinterval`; verify the getter-callback, callbacks-from-any-thread and poll-interval scenarios pass
- [ ] 4.2 Implement the polling-mode queue, `getStoredCommand[2]`, `putCommandAnswer[2]` and the 60 s timeout sweeper; verify the poll-and-answer, small-buffer and unanswered-request scenarios pass (timeout tested with a shortened test-only interval)
- [ ] 4.3 Implement `updateParameter[2]` and the error code mapping table; verify the pushed-update and busy-hardware scenarios and a table-driven test over all mapped codes pass
- [ ] 4.4 Implement the no-op status display hook for `bGUI` and `SECoP_S_showStatusWindow`; verify the GUI-requested scenario passes
- [ ] 4.5 Add a stress test with eight threads mixing pushes, polls and answers for 10 s; verify no crash, deadlock or unanswered request on all three targets

## 5. Conformance, docs and integration

- [ ] 5.1 Port the scenarios of SHALL's `tst_contextid.cpp` and `tst_protocol.cpp` to C tests against must (adapting expected wire output to SECoP 2.0); verify they pass on all three targets
- [ ] 5.2 Add a runtime-binding test (`LoadLibrary`/`dlopen` + symbol lookup by name, cdecl calls) for `SHALLserver32.dll` on i686; verify it passes
- [ ] 5.3 Write `COMPATIBILITY.md` (differences to SHALL: SECoP 2.0 wire protocol, removed C++ API, error class mapping, status TUI instead of Qt window) and a README section on replacing SHALL; verify the documented replacement steps work with the C example
- [ ] 5.4 Document a manual LabVIEW smoke test (32-bit and 64-bit) with a minimal VI calling init, create node, add module/parameter, complete, poll and answer; verify it once on a Windows machine with LabVIEW and record the result in the PR

## Workflow follow-up

- Archive after review; `add-status-tui` replaces the no-op status display hook.
