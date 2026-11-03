# Spec Delta

## Purpose

Lets SECoP clients receive a node's log messages over their connection, through the `logging` request and `log` events, at a level each client chooses per module.

## ADDED Requirements

### Requirement: Logging request
The node SHALL answer `logging <module>[:<parameter>] <level>` by setting the requesting connection's level for that module. It SHALL answer `logging  <level>`, or `logging <level>` with one space, by setting it for all modules. The reply SHALL be `logging <module> <level>`, or `logging  <level>` for the whole node, naming only the module even when a parameter was given.

#### Scenario: Whole node
- **WHEN** a client sends `logging  "debug"`
- **THEN** it receives `logging  "debug"` and gets log events of every module at all levels

#### Scenario: One module
- **WHEN** a client sends `logging t1 "error"`
- **THEN** it receives `logging t1 "error"`

#### Scenario: Parameter specifier
- **WHEN** a client sends `logging t1:target "info"`
- **THEN** it receives `logging t1 "info"`, and the level applies to all log events of `t1`

#### Scenario: Single space
- **WHEN** a client sends `logging "info"`
- **THEN** it receives `logging  "info"`

### Requirement: Logging levels
The node SHALL accept the levels `"debug"`, `"info"`, `"error"` and `"off"`, and `false` as `"off"`, echoing the value it received. Every module SHALL start at `"off"` for every connection. A later request SHALL replace the level for the modules it names.

#### Scenario: Default off
- **WHEN** a connection has not sent `logging`
- **THEN** it receives no `log` events

#### Scenario: False
- **WHEN** a client sends `logging t1 false`
- **THEN** it receives `logging t1 false` and no further log events of `t1`

#### Scenario: Module overrides node
- **WHEN** a client sends `logging  "error"` and then `logging t1 "debug"`
- **THEN** it gets all log events of `t1` and only error events of other modules

#### Scenario: Node replaces modules
- **WHEN** a client sends `logging t1 "debug"` and then `logging  "off"`
- **THEN** it receives no further log events of any module

#### Scenario: Invalid level
- **WHEN** a client sends `logging t1 "verbose"` or `logging t1 3`
- **THEN** it receives `error_logging t1` with a `WrongType` error report and its levels are unchanged

#### Scenario: Unknown module
- **WHEN** a client sends `logging nope "info"`
- **THEN** it receives `error_logging nope` with a `NoSuchModule` error report

### Requirement: Log events
The node SHALL send each log record of a module to every connection whose level for that module admits it, as `log <module>:<level> <message>` with the message as a JSON string. Error and warning records SHALL be sent as `error`, info records as `info` and debug records as `debug`. `"error"` admits error, `"info"` adds info and `"debug"` admits all.

#### Scenario: Poll failure
- **WHEN** a client sent `logging t1 "error"` and polling `t1` fails with "no answer from controller"
- **THEN** the client receives `log t1:error "polling failed: ..."` naming that error

#### Scenario: Level filters
- **WHEN** a client sent `logging t1 "info"` and the handler of `t1` logs one debug and one info message
- **THEN** the client receives only the info message, as `log t1:info "<message>"`

#### Scenario: Only requesting connections
- **WHEN** client A sent `logging  "debug"`, client B is activated but sent no `logging`, and module `t1` logs a message
- **THEN** A receives the `log` event and B does not

#### Scenario: Records without module
- **WHEN** the node logs a record not tied to a module, such as a failed accept
- **THEN** no `log` event is sent

### Requirement: Log events never disconnect
The node SHALL drop a connection's log events while its outgoing buffer is over the slow-client limit, instead of closing the connection, and SHALL keep sending its replies and updates as before.

#### Scenario: Slow client with logging
- **WHEN** a client with `logging  "debug"` stops reading and the node keeps logging
- **THEN** the connection is not closed because of log events, and it receives no log event that was produced while its buffer was over the limit

### Requirement: Application log records
The library SHALL let a module's handler log a message at level error, warning, info or debug for its module. Error, warning and info records SHALL also reach the node's log records like the node's own: the status TUI, monitoring and log files. Debug records SHALL reach only remote clients and `tracing`.

#### Scenario: Handler logs
- **WHEN** the handler of `t1` logs "heater relay switched" at info while a client has `logging t1 "info"`
- **THEN** the client receives `log t1:info "heater relay switched"`, and the record appears in the node's log file

#### Scenario: Debug record
- **WHEN** the handler of `t1` logs at debug
- **THEN** clients with `logging t1 "debug"` receive it, and it does not appear in the log file or the status TUI
