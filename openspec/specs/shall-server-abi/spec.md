# shall-server-abi Specification

## Purpose

Provides SHALL's `SECoP_S_*` C functions so that C and LabVIEW programs written for SHALL's ServerLib can build and run SEC nodes with must unchanged.

## Requirements

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
- **WHEN** `SECoP_S_nodeComplete` is called for a node with a parameter without datainfo
- **THEN** it returns an error code (e.g. `SECoP_S_ERROR_MISSING_MANDATORY`), the port is not opened, and `SECoP_S_showErrors` prints the problem

### Requirement: Warnings instead of failures
The library SHALL report recoverable problems as SHALL's positive warning codes (e.g. `SECoP_S_WARNING_NO_DESCRIPTION`, `SECoP_S_WARNING_CUSTOM_PROPERTY`, `SECoP_S_WARNING_MISS_PROPERTIES`) and continue, where SHALL did.

#### Scenario: Custom property without underscore
- **WHEN** `SECoP_S_addPropertyString("myprop", "x", ctx)` is called on a module
- **THEN** it returns `SECoP_S_WARNING_CUSTOM_PROPERTY` and, as in SHALL, the property is published under the given name `myprop`

#### Scenario: Automatic defaults
- **WHEN** a module, parameter or command has no description, or a command no datainfo
- **THEN** `SECoP_S_nodeComplete` succeeds with `SECoP_S_WARNING_MISS_PROPERTIES`, using SHALL's defaults (`"module without description"`, `{"type":"command"}`)

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
- **THEN** it returns `SECoP_S_WARNING_BUFFER_TOO_SMALL`, hands the request out anyway, writes a NUL-terminated prefix and reports the name's length, as SHALL does

#### Scenario: Unanswered request
- **WHEN** a polled request is not answered within 60 s
- **THEN** the client receives an error reply with `TimeoutError` and a later answer with that id returns `SECoP_S_ERROR_INVALID_NAME`

### Requirement: Pushed updates
The library SHALL send values passed to `SECoP_S_updateParameter[2]` (name `node:module:parameter`, value, optional sigma, timestamp or NaN) as updates to activated clients and store them for initial updates.

#### Scenario: Update with NaN timestamp
- **WHEN** `SECoP_S_updateParameter2("my_node:mod1:value", "3.5", 3, NULL, 0, NAN)` is called
- **THEN** activated clients receive `update mod1:value [3.5, {"t": <now>}]`

### Requirement: Error code mapping
The library SHALL turn a `SECoP_S_error` returned by an application callback or answer into the SECoP 2.0 error class given by the mapping table in the design (e.g. `SECoP_S_ERROR_READONLY` → `ReadOnly`, `SECoP_S_ERROR_IS_BUSY` → `IsBusy`, `SECoP_S_ERROR_OUT_OF_RANGE` → `RangeError`), and a code that the request's context registered with `SECoP_S_registerUserError` into its registered class name and text.

#### Scenario: Busy hardware
- **WHEN** a setter returns `SECoP_S_ERROR_IS_BUSY`
- **THEN** the client receives `error_change` with error class `IsBusy`

#### Scenario: Registered code
- **WHEN** a setter returns a code registered as `_Overheated`
- **THEN** the client receives `error_change` with error class `_Overheated`

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

### Requirement: Functions beyond SHALL
The library SHALL export `SECoP_S_filterStoredCommand`, `SECoP_S_filterStoredCommand2`, `SECoP_S_registerUserError` and `SECoP_S_unregisterUserError`, with the argument lists SHALL-LabVIEW's VIs pass, each plus a trailing `szContextID`. `SECoP-exports.h` SHALL declare them and mark them as additions that SHALL lacks.

#### Scenario: Declarations
- **WHEN** a C99 or C++11 program that calls the four functions with the argument lists below is compiled against the headers and linked with `SHALLserver`
- **THEN** it compiles and links:
  `SECoP_S_filterStoredCommand(const char* szFilterAccessibles, int iFilterAction, unsigned long long* pllId, enum SECoP_S_action* piAction, char* szAccessible, int* piAccessibleSize, CSECoPbaseType** ppValue, const char* szContextID)`,
  `SECoP_S_filterStoredCommand2(const char* szFilterAccessibles, int iFilterAction, unsigned long long* pllId, enum SECoP_S_action* piAction, char* szAccessible, int* piAccessibleSize, char* szValue, int* piValueSize, const char* szContextID)`,
  `SECoP_S_registerUserError(const char* szSECoPerror, const char* szMessageText, const char* szContextID)` and
  `SECoP_S_unregisterUserError(int iError, const char* szContextID)`, all returning `enum SECoP_S_error`

### Requirement: Filtered polled requests
`SECoP_S_filterStoredCommand[2]` SHALL work like `SECoP_S_getStoredCommand[2]` for the context `szContextID`, but SHALL hand out the oldest queued request of that context that matches the filter. It SHALL return `SECoP_S_ERROR_NO_DATA` when no request matches and leave the other requests queued. A request handed out this way SHALL be answered with `SECoP_S_putCommandAnswer[2]`.

#### Scenario: Only the selected accessibles
- **WHEN** requests `read mod1:a`, then `change mod1:b 5` are queued and the application calls `SECoP_S_filterStoredCommand2("my_node:mod1:b", 0, ..., "ctx")` for the node's context `ctx`
- **THEN** it gets the `change` of `my_node:mod1:b` with value `5`, and `read mod1:a` stays queued for the next call of `SECoP_S_getStoredCommand2`

#### Scenario: Nothing matches
- **WHEN** only `read mod1:a` is queued and the filter is `my_node:mod1:b`
- **THEN** the call returns `SECoP_S_ERROR_NO_DATA` and `read mod1:a` stays queued

#### Scenario: Action filter
- **WHEN** `read mod1:b`, then `change mod1:b 5` are queued and the filter is `my_node:mod1:b` with action `SECoP_S_ACTION_CHANGE` (2)
- **THEN** the call hands out the `change`, and the `read` stays queued

#### Scenario: Other contexts untouched
- **WHEN** contexts `a` and `b` each run a polling-mode node with a queued request, and the application calls the function for context `a` with an empty filter
- **THEN** it gets `a`'s request, and `b`'s request stays queued

#### Scenario: Unknown context
- **WHEN** the context ID names no context
- **THEN** the call returns `SECoP_S_ERROR_INVALID_CONTEXT_ID`, as `SECoP_S_getStoredCommand[2]` does

#### Scenario: Small buffers
- **WHEN** the accessible name or value buffer is too small
- **THEN** the call returns `SECoP_S_WARNING_BUFFER_TOO_SMALL` and hands the request out anyway, as `SECoP_S_getStoredCommand[2]` does

### Requirement: Filter syntax
`szFilterAccessibles` SHALL be a list of entries separated by commas, semicolons or whitespace. An entry SHALL be `node`, `node:module` or `node:module:accessible` and match every accessible it names, ignoring case. A NULL or empty list SHALL match every accessible. `iFilterAction` SHALL be `0` (any action) or a `SECoP_S_action` code.

#### Scenario: Module entry
- **WHEN** the filter is `my_node:mod1`
- **THEN** requests to every accessible of `mod1` match, and requests to `mod2` do not

#### Scenario: Several entries
- **WHEN** the filter is `"my_node:mod1:a\tmy_node:mod2:b\r\n"` (a tab-separated list with a line end)
- **THEN** requests to `mod1:a` and `mod2:b` match

#### Scenario: Any accessible
- **WHEN** the filter is empty and the action is `SECoP_S_ACTION_READ` (1)
- **THEN** the oldest queued `read` of the context is handed out

#### Scenario: Malformed filter
- **WHEN** an entry has more than three parts or a part that is not a valid SECoP identifier
- **THEN** the call returns `SECoP_S_ERROR_INVALID_NAME` and hands nothing out

#### Scenario: Unknown action
- **WHEN** `iFilterAction` is 4 or -1
- **THEN** the call returns `SECoP_S_ERROR_INVALID_VALUE` and hands nothing out

### Requirement: User error codes
`SECoP_S_registerUserError` SHALL take an error class name (a SECoP identifier), a text and an existing context, and return a new code below -999 that belongs to that context. `SECoP_S_unregisterUserError` SHALL remove a code of the given context. The library SHALL never reuse a code within the process. `SECoP_S_doneLibrary` with `bNodeOnly` 0 SHALL remove the context's codes.

#### Scenario: Register and answer
- **WHEN** `SECoP_S_registerUserError("_Overheated", "coil too hot", "ctx")` returns code `c`, and a polled `change mod1:target` of context `ctx` is answered with `SECoP_S_putCommandAnswer2(id, c, ...)`
- **THEN** the client receives `error_change mod1:target ["_Overheated", "target: coil too hot", {...}]` (the text prefixed with the accessible's name, as for SHALL's codes)

#### Scenario: From a callback
- **WHEN** a getter of a node of context `ctx` sets `*piError` to a code registered for `ctx`
- **THEN** the client receives an error report with that code's class name and text

#### Scenario: Standard class name
- **WHEN** `HardwareError` is registered with the text `sensor disconnected` and a request is answered with its code
- **THEN** the client receives a `HardwareError` error report with that text

#### Scenario: Registering a name again
- **WHEN** `_Overheated` is registered again for the same context, with the text `coil too hot!`
- **THEN** the call returns the same code, and later error reports use the new text

#### Scenario: Same name in two contexts
- **WHEN** contexts `a` and `b` both register `_Overheated`, with different texts
- **THEN** they get different codes, and each code's error reports use its own context's text

#### Scenario: Code of another context
- **WHEN** a request of context `a` is answered with a code that context `b` registered
- **THEN** the client receives an `InternalError` report, as for an unknown code

#### Scenario: Name differing only in case
- **WHEN** `_overheated` is registered for a context that has `_Overheated`, or `hardwareerror` is registered
- **THEN** the call returns `SECoP_S_ERROR_NAME_ALREADY_USED`

#### Scenario: Invalid name
- **WHEN** the name is NULL, empty, or not a SECoP identifier (e.g. `1st error` or `over-heated`)
- **THEN** the call returns `SECoP_S_ERROR_INVALID_NAME`

#### Scenario: Unknown context
- **WHEN** `SECoP_S_registerUserError` gets a context ID that `SECoP_S_initLibrary` or `SECoP_S_createNode[2]` has not created
- **THEN** it returns `SECoP_S_ERROR_INVALID_CONTEXT_ID`

#### Scenario: Unregister
- **WHEN** a code is unregistered for the context that registered it
- **THEN** `SECoP_S_unregisterUserError` returns `SECoP_S_SUCCESS`, an answer with that code gives an `InternalError` report, and registering the name again returns a new code

#### Scenario: Unregister for another context
- **WHEN** `SECoP_S_unregisterUserError` gets a code registered by another context
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE` and the code stays registered

#### Scenario: Unknown code
- **WHEN** `SECoP_S_unregisterUserError` gets a code that is not registered, including SHALL's own codes
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE`

#### Scenario: Codes end with the context
- **WHEN** `SECoP_S_doneLibrary(0, "ctx")` is called
- **THEN** the codes of `ctx` are removed; with `SECoP_S_doneLibrary(1, "ctx")` they stay

#### Scenario: Error string
- **WHEN** `SECoP_S_getErrorString` is called with a registered code
- **THEN** it returns `<name>: <text>`, and the returned string stays valid until the process ends, also after the code is removed
