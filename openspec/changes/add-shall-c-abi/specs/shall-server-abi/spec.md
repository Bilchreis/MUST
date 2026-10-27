# Spec Delta

## Purpose

Provides SHALL's `SECoP_S_*` C functions so that C and LabVIEW programs written for SHALL's ServerLib can build and run SEC nodes with must unchanged.

## ADDED Requirements

### Requirement: Exported server functions
The library SHALL export, with SHALL's exact names, signatures and `SECoP_S_error`/`SECoP_S_action` values, every function of SHALL's `SECoP-exports.h` and the callback types `SECoP_S_getsetFunction` and `SECoP_S_callFunction`.

#### Scenario: Symbol list
- **WHEN** the exported symbols of `SHALLserver64` are listed
- **THEN** they contain every `SECoP_S_*` function of SHALL's header, such as `SECoP_S_initLibrary`, `SECoP_S_createNode2`, `SECoP_S_getStoredCommand2` and `SECoP_S_putCommandAnswer2`

#### Scenario: Error strings
- **WHEN** `SECoP_S_getErrorString(SECoP_S_ERROR_READONLY)` is called
- **THEN** it returns a static, non-NULL description

### Requirement: Library initialization and context IDs
The library SHALL require `SECoP_S_initLibrary` before other calls (else `SECoP_S_ERROR_NOT_INITIALIZED`), accept and ignore its `QApplication*` argument, track each context ID separately, and make `SECoP_S_doneLibrary` stop only the nodes created under the given context.

#### Scenario: Not initialized
- **WHEN** `SECoP_S_createNode` is called before `SECoP_S_initLibrary`
- **THEN** it returns `SECoP_S_ERROR_NOT_INITIALIZED`

#### Scenario: Done for one context
- **WHEN** contexts `id1` and `id2` each created a node and `SECoP_S_doneLibrary(1, "id2")` is called
- **THEN** the node of `id2` stops and its port closes, while the node of `id1` keeps serving

### Requirement: Building nodes by focus
The library SHALL build nodes like SHALL: `SECoP_S_createNode[2]` starts a node, `SECoP_S_addModule`, `SECoP_S_addCommand`, `SECoP_S_add*Parameter` and `SECoP_S_addProperty*` apply to the most recently added item of the same context, `SECoP_S_setAddFocus` selects an item by `node:module:accessible`, and `SECoP_S_nodeComplete` validates and opens the TCP port.

#### Scenario: Example from SHALL's documentation
- **WHEN** the polling example from SHALL's `SECoP.h` (node `my_node`, module `mod1`, readable `value` with description and datainfo) is run
- **THEN** `SECoP_S_nodeComplete` succeeds and a client can `describe` the node on port 2055

#### Scenario: Two contexts build at the same time
- **WHEN** two threads build nodes under contexts `id1` and `id2` with interleaved calls
- **THEN** each property lands on the item of its own context

#### Scenario: Validation failure
- **WHEN** `SECoP_S_nodeComplete` is called for a node whose module has no description
- **THEN** it returns an error code (e.g. `SECoP_S_ERROR_MISSING_MANDATORY`), the port is not opened, and `SECoP_S_showErrors` prints the problem

### Requirement: Warnings instead of failures
The library SHALL report recoverable problems as SHALL's positive warning codes (e.g. `SECoP_S_WARNING_NO_DESCRIPTION`, `SECoP_S_WARNING_CUSTOM_PROPERTY`, `SECoP_S_WARNING_MISS_PROPERTIES`) and continue, where SHALL did.

#### Scenario: Custom property without underscore
- **WHEN** `SECoP_S_addPropertyString("myprop", "x", ctx)` is called on a module
- **THEN** it returns `SECoP_S_WARNING_CUSTOM_PROPERTY` and, as in SHALL, the property is published under the given name `myprop`

#### Scenario: Poll interval property
- **WHEN** `SECoP_S_addPropertyDouble("pollinterval", 2.0, ctx)` is called on a module in callback mode
- **THEN** the module's readable parameters are read through their getters about every 2 s and the results sent as updates

### Requirement: Callback mode
The library SHALL, when function pointers are enabled, answer `read`, `change` and `do` by calling the registered getter, setter or command function with the full name `node:module:accessible`, and send the returned value, sigma, timestamp or error to the client.

#### Scenario: Getter callback
- **WHEN** a client reads `mod1:value` and the getter sets the value 4.2 and `SECoP_S_SUCCESS`
- **THEN** the client receives `reply mod1:value [4.2, {...}]`

#### Scenario: Callbacks from any thread
- **WHEN** a callback blocks for 1 s
- **THEN** requests to other modules of the node are still answered meanwhile

### Requirement: Polling mode
The library SHALL, when function pointers are disabled, queue each `read`, `change` and `do` per context, hand it out once through `SECoP_S_getStoredCommand[2]` (returning `SECoP_S_ERROR_NO_DATA` when empty), and reply to the client when `SECoP_S_putCommandAnswer[2]` is called with its id.

#### Scenario: Poll and answer
- **WHEN** a client sends `change mod1:target 500` and the application calls `SECoP_S_getStoredCommand2` for its context
- **THEN** it gets an id, `SECoP_S_ACTION_CHANGE`, the name `my_node:mod1:target` and the value `500`, and after `SECoP_S_putCommandAnswer2` with that id the client receives `changed`

#### Scenario: Small buffers
- **WHEN** `SECoP_S_getStoredCommand2` is called with a name buffer too small for the accessible name
- **THEN** it returns `SECoP_S_WARNING_BUFFER_TOO_SMALL`, writes a NUL-terminated prefix and reports the needed size

#### Scenario: Unanswered request
- **WHEN** a polled request is not answered within 60 s
- **THEN** the client receives an error reply with `TimeoutError` and a later answer with that id returns an error code

### Requirement: Pushed updates
The library SHALL send values passed to `SECoP_S_updateParameter[2]` (name `node:module:parameter`, value, optional sigma, timestamp or NaN) as updates to activated clients and store them for initial updates.

#### Scenario: Update with NaN timestamp
- **WHEN** `SECoP_S_updateParameter2("my_node:mod1:value", "3.5", 3, NULL, 0, NAN)` is called
- **THEN** activated clients receive `update mod1:value [3.5, {"t": <now>}]`

### Requirement: Error code mapping
The library SHALL turn a `SECoP_S_error` returned by an application callback or answer into the SECoP 2.0 error class given by the mapping table in the design (e.g. `SECoP_S_ERROR_READONLY` → `ReadOnly`, `SECoP_S_ERROR_IS_BUSY` → `IsBusy`, `SECoP_S_ERROR_OUT_OF_RANGE` → `RangeError`).

#### Scenario: Busy hardware
- **WHEN** a setter returns `SECoP_S_ERROR_IS_BUSY`
- **THEN** the client receives `error_change` with error class `IsBusy`

### Requirement: Qt-related arguments
The library SHALL accept `SECoP_S_setManyThreads` and the `bGUI` flag without error; `bGUI` and `SECoP_S_showStatusWindow` request the status display, which has no effect while no status display is available.

#### Scenario: GUI requested without status display
- **WHEN** `SECoP_S_initLibrary(NULL, 1, 1, "ctx")` is called before the status TUI exists
- **THEN** it returns `SECoP_S_SUCCESS` and the node works normally

### Requirement: Thread safety
The library SHALL allow every `SECoP_S_*` and `SECoP_V_*` function to be called from any thread at any time.

#### Scenario: Concurrent calls
- **WHEN** eight threads push updates and poll requests concurrently for 10 s
- **THEN** no call crashes or deadlocks and every request is answered
