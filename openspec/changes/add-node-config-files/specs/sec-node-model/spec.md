# Spec Delta

## ADDED Requirements

### Requirement: Inspecting and adjusting a declared module
Before a module is added to a node, the library SHALL let an application look up the datainfo of a parameter declared on it by name, and set the initial value of a declared cached parameter or `pollinterval` parameter by name. Setting it for an undeclared parameter, for one that is neither cached nor `pollinterval`, or with a value that violates the parameter's datainfo, including its limits, SHALL be rejected with an error naming the parameter.

#### Scenario: Initial value after declaration
- **WHEN** a module declares a cached `pollinterval` with initial value 1.0 and the application then sets its initial value to 2.0 by name
- **THEN** after completion and start, a `read` of `pollinterval` returns 2.0

#### Scenario: Not a cached parameter
- **WHEN** the application sets the initial value of the module's non-cached `value` parameter by name
- **THEN** the call is rejected with an error naming `value`

#### Scenario: Unknown parameter
- **WHEN** the application asks for the datainfo of `ramp` on a module that declares no `ramp`
- **THEN** no datainfo is returned
