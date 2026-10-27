# LabVIEW smoke test

A manual check that LabVIEW can use must in place of SHALL. CI cannot run LabVIEW; run
this once per release on Windows, with 32-bit and 64-bit LabVIEW, and record the result
in the pull request.

## Prepare

1. Download the CI artifact `must-shall-i686-pc-windows-msvc` (for 32-bit LabVIEW) or
   `must-shall-x86_64-pc-windows-msvc` (64-bit), or build it with
   `cargo xtask dist --target <triple>`.
2. Copy `SHALLserver32.dll` and `SHALLvariant32.dll` (or `…64.dll`) into one folder, e.g.
   next to the VI. Make sure no SHALL or Qt DLLs are on the search path.
3. Have a SECoP client ready, e.g. Frappy's GUI or a plain TCP client such as
   `nc localhost 2055` / PuTTY (raw, port 2055).

## The VI

Build a VI with "Call Library Function" nodes (calling convention **C**, library
`SHALLserver32.dll` or `SHALLserver64.dll`), in this order:

| Step | Function | Parameters | Expected return |
|---|---|---|---|
| 1 | `SECoP_S_initLibrary` | `pApplication` = 0 (pointer-sized integer), `bGUI` = 0, `bEnableFunctionPointers` = 0, `szContextID` = `"lv"` (C string) | 0 |
| 2 | `SECoP_S_createNode` | `"lv_node"`, `"LabVIEW smoke test"`, port 2055 (U16), `"lv"` | 0 |
| 3 | `SECoP_S_addModule` | `"mod1"`, `"lv"` | 0 |
| 4 | `SECoP_S_addPropertyString` | `"description"`, `"test module"`, `"lv"` | 0 |
| 5 | `SECoP_S_addWritableParameter` | `"target"`, 0, 0 (pointer-sized), `"lv"` | 0 |
| 6 | `SECoP_S_addPropertyString` | `"datainfo"`, `{"type":"double","unit":"K"}`, `"lv"` | 0 |
| 7 | `SECoP_S_nodeComplete` | `"lv"` | ≥ 0 (4 = warning for missing `interface_classes`) |
| 8 | loop: `SECoP_S_getStoredCommand2` | `pllId` (U64 pointer), `piAction` (I32 pointer), name buffer (C string, 100 bytes) and its size (I32 pointer = 100), value buffer (100 bytes) and its size (I32 pointer = 100), `"lv"` | -10 (no data) until a request arrives, then 0 |
| 9 | on 0: `SECoP_S_putCommandAnswer2` | the id, 0, the value buffer, its length, empty string, 0, 0.0 | 0 |
| 10 | on stop: `SECoP_S_doneLibrary` | 0, `"lv"` | – |

## Check

1. Run the VI. From the client send `*IDN?`: the reply starts with `ISSE`.
2. Send `describe`: the description lists `mod1` with `target`.
3. Send `change mod1:target 300`: the VI's loop shows action 2 and name
   `lv_node:mod1:target` with value `300.0`; the client receives
   `changed mod1:target [300.0,{"t":…}]`.
4. Stop the VI (step 10): the client's connection closes. Run the VI again: it starts on
   the same port without error.
5. Close LabVIEW: it exits without hanging or a crash report.

## Results

| Date | LabVIEW version, bitness | must commit | Result | Notes |
|---|---|---|---|---|
| | | | | |
