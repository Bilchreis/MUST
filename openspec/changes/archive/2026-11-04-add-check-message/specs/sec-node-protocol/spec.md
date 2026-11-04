# Spec Delta

## ADDED Requirements

### Requirement: Check
The node SHALL answer `check <module>:<accessible> [<value>]` without changing any parameter, cache entry or hardware state. An accessible that is not marked `checkable: true` SHALL get `NotCheckable`. Otherwise the value SHALL be validated as for `change` (a parameter, including dynamic limits) or `do` (a command's argument) before the module's handler decides.

#### Scenario: Accepted value
- **WHEN** `t1:target` is checkable, its handler accepts 250, and a client sends `check t1:target 250`
- **THEN** it receives `checked t1:target [250, {"t": <time>}]`, `t1:target` keeps its value, and no `update` is sent

#### Scenario: Not checkable
- **WHEN** a client sends `check t1:target 5` and `t1:target` has no `checkable` property
- **THEN** it receives `error_check t1:target` with a `NotCheckable` error report

#### Scenario: Rejected by the handler
- **WHEN** `t1:target` is checkable and its handler rejects 250 with `Impossible` "heater off in this configuration"
- **THEN** the client receives `error_check t1:target ["Impossible", "heater off in this configuration", {...}]`

#### Scenario: Value outside datainfo
- **WHEN** `t1:target` is checkable with `max` 300 and a client sends `check t1:target 400`
- **THEN** it receives `error_check t1:target` with a `RangeError` error report and the handler is not asked

#### Scenario: Value beyond dynamic limit
- **WHEN** `t1:target` is checkable, `t1:target_max` is 200, and a client sends `check t1:target 250`
- **THEN** it receives `error_check t1:target` with a `RangeError` error report and the handler is not asked

#### Scenario: Readonly parameter
- **WHEN** a client sends `check t1:value 5` for the readonly parameter `t1:value`
- **THEN** it receives `error_check t1:value` with a `ReadOnly` error report

#### Scenario: Command argument
- **WHEN** the command `t1:ramp` with a double argument is checkable and a client sends `check t1:ramp "fast"`
- **THEN** it receives `error_check t1:ramp` with a `WrongType` error report

#### Scenario: Unknown accessible
- **WHEN** a client sends `check t1:nope 1`
- **THEN** it receives `error_check t1:nope` with a `NoSuchParameter` error report

### Requirement: Check replies
The node SHALL send `checked` and `error_check` replies only to the connection that sent the `check`, never to other activated clients, and SHALL apply the node's handler timeout to checks as to other requests.

#### Scenario: Other clients see nothing
- **WHEN** client A sends `check t1:target 250` while client B is activated
- **THEN** A receives `checked t1:target ...` and B receives no message

#### Scenario: Hanging check
- **WHEN** a check handler never returns and the node timeout is 2 s
- **THEN** the client receives `error_check t1:target` with `TimeoutError` after about 2 s

### Requirement: Empty lines
The node SHALL ignore empty lines.

#### Scenario: Empty line between requests
- **WHEN** a client sends an empty line followed by `ping 1`
- **THEN** it receives only `pong 1 ...`

### Requirement: Logging not supported
The node SHALL answer `logging` requests with an `error_logging` reply carrying a `ProtocolError`.

#### Scenario: Logging request
- **WHEN** a client sends `logging t1 "debug"`
- **THEN** it receives `error_logging t1` with a `ProtocolError` error report

## REMOVED Requirements

### Requirement: Optional and custom messages
**Reason**: Its scenario "Check not supported" no longer holds. "Check" and "Check replies" now cover `check`, "Logging not supported" covers `logging`, and "Empty lines" covers empty lines.
**Migration**: Clients that treated `error_check` with `ProtocolError` as "unsupported" now get `NotCheckable` for accessibles without `checkable: true`, as SECoP 2.0 prescribes.
