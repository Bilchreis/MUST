# Spec Delta

## ADDED Requirements

### Requirement: Poll configuration of C modules
The library SHALL decide the polling of a module built through the C ABI, in callback and in polling mode, as follows:
- **`pollinterval` parameter:** when the module declares a parameter `pollinterval` with a `double` datainfo without a negative `min`, the module SHALL poll by the default tiers of `sec-node-runtime` (fast tier `value` and `status` of a `Readable`, `Writable` or `Drivable` module, every other parameter every 60 s), with that parameter as the fast interval (5 s unless the host gives it another initial value). SHALL's `pollinterval` properties of the module and of its parameters SHALL be ignored and reported as warnings by `SECoP_S_showErrors`. A change of the parameter goes to the setter callback or polling-mode request first, and the answered value re-times polling.
- **`pollinterval` properties, no such parameter:** SHALL's behavior. The module property `pollinterval` (seconds, 0 for none) polls all non-constant, non-cached parameters at that interval; the parameter property gives that parameter its own interval, which wins over the module property; when only parameter properties are set, only those parameters are polled.
- **Neither:** the default tiers of `sec-node-runtime` (fast 5 s, slow 60 s).

#### Scenario: Module property polls all parameters
- **WHEN** a `Drivable` module built through the C ABI has `SECoP_S_addPropertyDouble("pollinterval", 2.0, ctx)` and parameters `value`, `status`, `target` and `ramp`
- **THEN** all four are read through their getters about every 2 s

#### Scenario: Parameter property wins
- **WHEN** that module's `ramp` parameter has the property `pollinterval` 10.0
- **THEN** `ramp` is read about every 10 s and the others about every 2 s

#### Scenario: Only parameter properties
- **WHEN** a module sets no module property and only its `ramp` parameter has the property `pollinterval` 1.0
- **THEN** only `ramp` is read, about every second

#### Scenario: Declared pollinterval parameter
- **WHEN** a `Drivable` module declares a writable `double` parameter `pollinterval`, also has the module property `pollinterval` 2.0, and a client changes the parameter to 0.5
- **THEN** a warning about the ignored property is reported, the setter callback is called with 0.5, and after it succeeds `value` and `status` are read about every 0.5 s while `target` and `ramp` are read about every 60 s

#### Scenario: No poll configuration
- **WHEN** a `Drivable` module sets no `pollinterval` property and no `pollinterval` parameter
- **THEN** `value` and `status` are read about every 5 s and `target` and `ramp` about every 60 s

#### Scenario: Polling mode
- **WHEN** a module in polling mode sets no `pollinterval` property and no `pollinterval` parameter
- **THEN** the host receives a stored `read` request for `value` and for `status` about every 5 s through `SECoP_S_getStoredCommand`
