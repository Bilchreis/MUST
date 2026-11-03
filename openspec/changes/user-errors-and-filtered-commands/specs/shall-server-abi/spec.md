# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Error code mapping
The library SHALL turn a `SECoP_S_error` returned by an application callback or answer into the SECoP 2.0 error class given by the mapping table in the design (e.g. `SECoP_S_ERROR_READONLY` → `ReadOnly`, `SECoP_S_ERROR_IS_BUSY` → `IsBusy`, `SECoP_S_ERROR_OUT_OF_RANGE` → `RangeError`), and a code that the request's context registered with `SECoP_S_registerUserError` into its registered class name and text.

#### Scenario: Busy hardware
- **WHEN** a setter returns `SECoP_S_ERROR_IS_BUSY`
- **THEN** the client receives `error_change` with error class `IsBusy`

#### Scenario: Registered code
- **WHEN** a setter returns a code registered as `_Overheated`
- **THEN** the client receives `error_change` with error class `_Overheated`
