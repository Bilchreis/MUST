# Spec Delta

## MODIFIED Requirements

### Requirement: Module handlers
The library SHALL route `read`, `change`, `do` and `check` requests for a module to an application-provided handler for that module, which returns a value with optional timestamp and uncertainty (for `check`: acceptance), or a SECoP error class with a message. A handler that does not implement `check` SHALL answer it with `NotImplemented`.

#### Scenario: Handler error reaches the client
- **WHEN** the handler fails a read of `t1:value` with `CommunicationFailed` "no answer from controller"
- **THEN** the client receives `error_read t1:value` with that error class and text

#### Scenario: Check not implemented by the handler
- **WHEN** `t1:target` is declared `checkable: true` but the module's handler does not implement `check`
- **THEN** a client sending `check t1:target 5` receives `error_check t1:target` with a `NotImplemented` error report

#### Scenario: Check counts as a request of the module
- **WHEN** a slow `change` of `t1:target` is in progress and the same module receives a `check`
- **THEN** the check handler is called only after the change handler has finished, as for any other request of the module
