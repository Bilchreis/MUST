# Spec Delta

## ADDED Requirements

### Requirement: Module start hook
The library SHALL call each module handler's start hook once when the node starts, before that module's startup reads and polling and before any request to it is handled. Requests to the module that arrive meanwhile SHALL wait for the start hook. Its failure, or its not finishing within ten times the node's `timeout`, SHALL be logged with the module name. Then the module SHALL go on with its startup reads, polling and requests. A handler without a start hook SHALL behave as before.

#### Scenario: Read waits for start
- **WHEN** a module's start hook takes 1 s and a client sends `read T:value` right after connecting
- **THEN** the read handler is called only after the start hook has finished, and the client receives the value

#### Scenario: Start hook fails
- **WHEN** a module's start hook fails with "controller not found"
- **THEN** the node logs the error with the module name, keeps running, and handles requests to the module

#### Scenario: Hanging start hook
- **WHEN** a module's start hook never returns and the node `timeout` is 1 s
- **THEN** after about 10 s the hang is logged, and the module's requests are handled
