# Spec Delta

## MODIFIED Requirements

### Requirement: Data and error reports
The node SHALL send values as data reports `[<value>, <qualifiers>]` with `t` (timestamp) and `e` (uncertainty) qualifiers when known, and errors as error reports `[<class>, <text>, {"t": ...}]` using the SECoP 2.0 error class names, or the custom class name a handler gave the error.

#### Scenario: Error report format
- **WHEN** a read handler fails with a hardware error "sensor disconnected"
- **THEN** the reply is `error_read t1:value ["HardwareError", "sensor disconnected", {"t": <time>}]`

#### Scenario: Custom error class
- **WHEN** a read handler fails with an error of the custom class `_Overheated` and the text "coil too hot"
- **THEN** the reply is `error_read t1:value ["_Overheated", "coil too hot", {"t": <time>}]`
