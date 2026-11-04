# Spec Delta

## ADDED Requirements

### Requirement: Checks in callback mode
The library SHALL, when function pointers are enabled, answer a `check` of an accessible that has a check function by calling it with the full name `node:module:accessible` and the validated value. The returned `SECoP_S_error` SHALL be mapped like a setter's: success gives `checked`, an error code gives `error_check`.

#### Scenario: Check callback accepts
- **WHEN** `SECoP_S_setCheckFunction` was called for `mod1:target` and a client sends `check mod1:target 500`, and the function sets `SECoP_S_SUCCESS`
- **THEN** the function was called with `my_node:mod1:target` and a value of 500, the setter was not called, and the client receives `checked mod1:target [500, {...}]`

#### Scenario: Check callback rejects
- **WHEN** the check function sets `SECoP_S_ERROR_OUT_OF_RANGE`
- **THEN** the client receives `error_check mod1:target` with a `RangeError` error report

#### Scenario: Registered error class
- **WHEN** the application registered `Impossible` with `SECoP_S_registerUserError` and the check function sets that code
- **THEN** the client receives `error_check mod1:target` with error class `Impossible`

### Requirement: Checks in polling mode
The library SHALL, when function pointers are disabled, queue a `check` of an accessible marked `checkable: true` with action `SECoP_S_ACTION_CHECK` (4), and hand it out and match it like `read`, `change` and `do`. The client SHALL be answered when `SECoP_S_putCommandAnswer[2]` is called with its id. Any value passed with the answer SHALL be ignored.

#### Scenario: Poll and answer a check
- **WHEN** `mod1:target` has `checkable: true`, a client sends `check mod1:target 500`, and the application calls `SECoP_S_getStoredCommand2`
- **THEN** it gets an id, action 4, the name `my_node:mod1:target` and the value `500`, and after `SECoP_S_putCommandAnswer2` with that id and `SECoP_S_SUCCESS` the client receives `checked mod1:target [500, {...}]`

#### Scenario: Filter by the check action
- **WHEN** a `change` and a `check` of `mod1:target` are queued and the application calls `SECoP_S_filterStoredCommand2` with action 4
- **THEN** it gets the check, and the change stays queued

#### Scenario: Not checkable
- **WHEN** a client sends `check mod1:target 500` and `mod1:target` has no `checkable` property
- **THEN** nothing is queued and the client receives `error_check mod1:target` with a `NotCheckable` error report

#### Scenario: Unanswered check
- **WHEN** a queued check is not answered within the polled request timeout
- **THEN** the client receives `error_check mod1:target` with `TimeoutError`

### Requirement: Check functions beyond SHALL
The library SHALL export `SECoP_S_setCheckFunction(SECoP_S_checkFunction, const char* szContextID)`, which sets the check function of the context's focused parameter or command and publishes `checkable: true` for it. `SECoP_S_nodeComplete` SHALL fail with `SECoP_S_ERROR_MISSING_MANDATORY` in callback mode for an accessible with `checkable: true` but no check function.

#### Scenario: Focus is not an accessible
- **WHEN** `SECoP_S_setCheckFunction` is called while the context's focus is a module
- **THEN** it returns `SECoP_S_ERROR_INVALID_PARAMETER` and nothing changes

#### Scenario: Checkable without function
- **WHEN** in callback mode `SECoP_S_addPropertyBoolean("checkable", 1, ctx)` is called on a parameter that has no check function, and then `SECoP_S_nodeComplete`
- **THEN** it returns `SECoP_S_ERROR_MISSING_MANDATORY`, the port is not opened, and `SECoP_S_showErrors` names the accessible

#### Scenario: Description shows checkable
- **WHEN** a check function was set for `mod1:target` and a client sends `describe`
- **THEN** the accessible `target` of `mod1` has `"checkable": true`

#### Scenario: Header declares the additions
- **WHEN** a C program includes `SECoP.h`
- **THEN** `SECoP_S_ACTION_CHECK`, `SECoP_S_checkFunction` and `SECoP_S_setCheckFunction` are declared, and the program compiles as C99 and C++11
