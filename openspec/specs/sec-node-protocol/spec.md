# sec-node-protocol Specification

## Purpose

Defines what a must SEC node sends and accepts on the wire: SECoP 2.0 message framing, the mandatory messages, and error replies.

## Requirements

### Requirement: Message framing
The node SHALL read messages as lines ending in LF with an optional preceding CR, split into action, optional specifier and optional JSON data at the first two spaces, and SHALL end every sent message with a single LF.

#### Scenario: CRLF line ending
- **WHEN** a client sends `read t1:value\r\n`
- **THEN** the node treats it like `read t1:value`

#### Scenario: Data containing spaces
- **WHEN** a client sends `change t1:target {"a": 1, "b": 2}`
- **THEN** the data part is `{"a": 1, "b": 2}`

### Requirement: Oversized and malformed messages
The node SHALL reply with a `ProtocolError` to a message longer than the configured maximum line length (default 1 MiB) or with an unknown action, and SHALL reply with `BadJSON` when the data part is not valid JSON.

#### Scenario: Unknown action
- **WHEN** a client sends `meas:volt?`
- **THEN** the node replies `error_meas:volt?` followed by a `ProtocolError` error report

#### Scenario: Invalid JSON
- **WHEN** a client sends `change t1:target {oops`
- **THEN** the node replies `error_change t1:target` with a `BadJSON` error report

### Requirement: Identification
The node SHALL reply to `*IDN?` with `ISSE,SECoP,<draft-date>,v2.0`, where `<draft-date>` is the date of the SECoP 2.0 draft must implements, and SHALL deactivate all updates on that connection.

#### Scenario: Identification resets activation
- **WHEN** an activated client sends `*IDN?`
- **THEN** it receives the identification line and no further unrequested updates

### Requirement: Description
The node SHALL reply to `describe` (with any or no specifier) with `describing . ` followed by the structure report on a single line.

#### Scenario: Describe
- **WHEN** a client sends `describe`
- **THEN** it receives one line starting with `describing . {` containing the structure report

### Requirement: Activation
The node SHALL answer `activate` by sending the current value of every non-constant parameter as `update` (or `error_update` if its last read failed) and then `active`; with a module specifier it SHALL do so for that module only and reply `active <module>`.

#### Scenario: Global activation
- **WHEN** a client sends `activate` to a node with module `t1` having `value` and `status`
- **THEN** it receives `update t1:value ...` and `update t1:status ...` before `active`

#### Scenario: Module-wise activation of an unknown module
- **WHEN** a client sends `activate nope`
- **THEN** the node replies `error_activate nope` with a `NoSuchModule` error report

### Requirement: Deactivation
The node SHALL answer `deactivate` with `inactive` and stop sending unrequested updates to that connection; with a module specifier it SHALL stop updates for that module only and reply `inactive <module>`.

#### Scenario: Module-wise deactivation
- **WHEN** a client activated globally sends `deactivate t1`
- **THEN** it receives `inactive t1` and no further updates for `t1`, but still for other modules

### Requirement: Heartbeat
The node SHALL answer `ping <token>` with `pong <token> [null, {"t": <now>}]`, and `ping` without token with `pong  [null, {"t": <now>}]` (two spaces).

#### Scenario: Ping with token
- **WHEN** a client sends `ping 42`
- **THEN** it receives `pong 42` followed by a data report with value null and a timestamp

### Requirement: Read
The node SHALL answer `read <module>:<parameter>` with `reply <module>:<parameter> <data-report>` holding a reasonably recent value, or with `error_read` and an error report.

#### Scenario: Unknown parameter
- **WHEN** a client sends `read t1:nope`
- **THEN** it receives `error_read t1:nope` with a `NoSuchParameter` error report

#### Scenario: Read of a command
- **WHEN** a client sends `read t1:stop` where `stop` is a command
- **THEN** it receives `error_read t1:stop` with a `NoSuchParameter` error report

### Requirement: Change
The node SHALL answer `change <module>:<parameter> <value>` by validating the value in request mode (including dynamic limits from the parameter's `_min`/`_max`/`_limits`), performing the change, and replying `changed` with the read-back value, or `error_change` with an error report.

#### Scenario: Readonly parameter
- **WHEN** a client sends `change t1:value 5`
- **THEN** it receives `error_change t1:value` with a `ReadOnly` error report

#### Scenario: Value beyond dynamic limit
- **WHEN** `t1:target_max` is 200 and a client sends `change t1:target 250`
- **THEN** it receives `error_change t1:target` with a `RangeError` error report and the hardware is not asked

### Requirement: Do
The node SHALL answer `do <module>:<command> [<argument>]` by validating the argument, executing the command and replying `done` with the result (null if none), or `error_do`; `do m:c` and `do m:c null` SHALL behave the same for a command without argument.

#### Scenario: Command without argument
- **WHEN** a client sends `do t1:stop null`
- **THEN** it receives `done t1:stop [null, {...}]`

#### Scenario: Do on a parameter
- **WHEN** a client sends `do t1:value`
- **THEN** it receives `error_do t1:value` with a `NoSuchCommand` error report

### Requirement: Data and error reports
The node SHALL send values as data reports `[<value>, <qualifiers>]` with `t` (timestamp) and `e` (uncertainty) qualifiers when known, and errors as error reports `[<class>, <text>, {"t": ...}]` using the SECoP 2.0 error class names.

#### Scenario: Error report format
- **WHEN** a read handler fails with a hardware error "sensor disconnected"
- **THEN** the reply is `error_read t1:value ["HardwareError", "sensor disconnected", {"t": <time>}]`

### Requirement: Optional and custom messages
The node SHALL answer `check` and `logging` requests with an `error_<action>` reply carrying a `ProtocolError`, and SHALL ignore empty lines.

#### Scenario: Check not supported
- **WHEN** a client sends `check t1:target 5`
- **THEN** it receives `error_check t1:target` with a `ProtocolError` error report
