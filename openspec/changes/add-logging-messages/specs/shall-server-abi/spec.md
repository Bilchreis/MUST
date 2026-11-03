# Spec Delta

## ADDED Requirements

### Requirement: Application log messages
The library SHALL export `SECoP_S_logMessage(const char* szModule, enum SECoP_S_logLevel iLevel, const char* szMessage)`, a must addition, which logs `szMessage` for the running node's module named `node:module` at `SECoP_S_LOG_ERROR` (1), `SECoP_S_LOG_WARNING` (2), `SECoP_S_LOG_INFO` (3) or `SECoP_S_LOG_DEBUG` (4), as an application log record of that module.

#### Scenario: Host logs for a module
- **WHEN** a client sent `logging mod1 "info"` and the application calls `SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_INFO, "cooling started")`
- **THEN** it returns `SECoP_S_SUCCESS` and the client receives `log mod1:info "cooling started"`

#### Scenario: From any thread
- **WHEN** a getter callback calls `SECoP_S_logMessage` for its own module while the node is answering the read
- **THEN** the call returns without waiting for the read and the read is answered

#### Scenario: Unknown module
- **WHEN** `SECoP_S_logMessage("my_node:nope", SECoP_S_LOG_INFO, "x")` is called
- **THEN** it returns `SECoP_S_ERROR_INVALID_MODULE` and nothing is logged

#### Scenario: Unknown node or node not complete
- **WHEN** `SECoP_S_logMessage` names a node that does not exist or has not completed `SECoP_S_nodeComplete`
- **THEN** it returns `SECoP_S_ERROR_INVALID_NODE`

#### Scenario: Invalid level or NULL text
- **WHEN** `SECoP_S_logMessage` is called with level 0 or 5, or with a NULL `szMessage`
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE` and nothing is logged

#### Scenario: Header declares the addition
- **WHEN** a C program includes `SECoP.h`
- **THEN** `SECoP_S_logLevel`, its four values and `SECoP_S_logMessage` are declared, and the program compiles as C99 and C++11
