# Spec Delta

## MODIFIED Requirements

### Requirement: Callback mode
The library SHALL, when function pointers are enabled, answer `read`, `change` and `do` by calling the registered getter, setter or command function with the full name `node:module:accessible`, and send the returned value, sigma, timestamp or error to the client. It SHALL call the functions of one module one at a time, while functions of different modules may run at the same time, as SHALL does with its default of one thread per module.

#### Scenario: Getter callback
- **WHEN** a client reads `mod1:value` and the getter sets the value 4.2 and `SECoP_S_SUCCESS`
- **THEN** the client receives `reply mod1:value [4.2, {...}]`

#### Scenario: Callbacks from any thread
- **WHEN** a callback blocks for 1 s
- **THEN** requests to other modules of the node are still answered meanwhile

#### Scenario: One module at a time
- **WHEN** the getter of `mod1:a` blocks for 1 s and another client reads `mod1:b`
- **THEN** the getter of `mod1:b` is called only after the getter of `mod1:a` has returned

### Requirement: Polling mode
The library SHALL, when function pointers are disabled, queue each `read`, `change` and `do` per context as it arrives, also while other requests of the same module are handed out and not yet answered, hand it out once through `SECoP_S_getStoredCommand[2]` (returning `SECoP_S_ERROR_NO_DATA` when empty), and reply to the client when `SECoP_S_putCommandAnswer[2]` is called with its id, in any order. A `read` of a parameter that is already queued or handed out SHALL not be queued again, unless it was queued before a `change` or `do` of the module was answered and the new `read` came from a client after that answer; a poll SHALL never queue a second read of a parameter. One answer SHALL reply to every client waiting for it, as in SHALL.

#### Scenario: Poll and answer
- **WHEN** a client sends `change mod1:target 500` and the application calls `SECoP_S_getStoredCommand2` for its context
- **THEN** it gets an id, `SECoP_S_ACTION_CHANGE`, the name `my_node:mod1:target` and the value `500`, and after `SECoP_S_putCommandAnswer2` with that id the client receives `changed`

#### Scenario: Small buffers
- **WHEN** `SECoP_S_getStoredCommand2` is called with a name buffer too small for the accessible name
- **THEN** it returns `SECoP_S_WARNING_BUFFER_TOO_SMALL`, hands the request out anyway, writes a NUL-terminated prefix and reports the name's length, as SHALL does

#### Scenario: Unanswered request
- **WHEN** a polled request is not answered within 60 s
- **THEN** the client receives an error reply with `TimeoutError` and a later answer with that id returns `SECoP_S_ERROR_INVALID_NAME`

#### Scenario: Requests of one module at once
- **WHEN** four clients read `mod1:p0` to `mod1:p3` at the same time
- **THEN** four calls of `SECoP_S_getStoredCommand2` in a row return all four requests before any of them is answered

#### Scenario: Answers in any order
- **WHEN** the application has fetched reads of `mod1:p0` and `mod1:p1` and answers `mod1:p1` first
- **THEN** the client reading `mod1:p1` receives its reply at once, without waiting for `mod1:p0`

#### Scenario: Reads of one parameter merged
- **WHEN** a fourth client has activated `mod1`, and three clients read `mod1:value` before the application fetches the request
- **THEN** the application gets one request; after it is answered the three clients each receive `reply mod1:value` with the same value, and the fourth client receives one `update mod1:value`

#### Scenario: Read after changed
- **WHEN** the application has fetched a read of `mod1:status` without answering it, then answers client B's `change mod1:target 5`, and B then reads `mod1:status`
- **THEN** the application gets a second request for `mod1:status`, and B's reply comes from its answer

#### Scenario: Poll of a queued parameter
- **WHEN** a client's read of `mod1:value` is queued or handed out and a poll tick falls due for `mod1:value`
- **THEN** no second request for `mod1:value` is queued, and the answer to the client's read is sent as the update

#### Scenario: Unanswered read does not block the module
- **WHEN** the application leaves a read of `mod1:p0` unanswered and a client sends `change mod1:target 5`
- **THEN** the application can fetch the change at once, and the client receives `changed` after it is answered

#### Scenario: Poll tick queued at once
- **WHEN** a poll tick of `mod1` falls due for `value`, `status` and `target`
- **THEN** all three reads are queued at once and can be fetched in one pass of the application's loop
