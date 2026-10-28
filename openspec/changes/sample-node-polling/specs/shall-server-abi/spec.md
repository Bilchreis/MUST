# Spec Delta

## ADDED Requirements

### Requirement: Poll interval property
The library SHALL treat SHALL's module property `pollinterval` (seconds, 0 for none) as a uniform interval for all non-constant, non-cached parameters of the module, and the parameter property `pollinterval` as that parameter's own interval, as in SHALL; the Rust API's tier inference SHALL NOT apply to modules built through the C ABI. A module that also declares a `double` parameter named `pollinterval` SHALL have its uniform interval driven by that parameter (see `sec-node-runtime`): a change goes to the setter callback or polling-mode request first, and the answered value re-times polling.

#### Scenario: Uniform polling kept
- **WHEN** a `Drivable` module built through the C ABI has `SECoP_S_addPropertyDouble("pollinterval", 2.0, ctx)` and parameters `value`, `status`, `target` and `ramp`
- **THEN** all four are read through their getters about every 2 s

#### Scenario: Parameter property wins
- **WHEN** that module's `ramp` parameter has the property `pollinterval` 10.0
- **THEN** `ramp` is read about every 10 s and the others about every 2 s

#### Scenario: Declared pollinterval parameter
- **WHEN** the module also declares a writable `double` parameter `pollinterval` and a client changes it to 0.5
- **THEN** the setter callback is called with 0.5 and, after it succeeds, the module's parameters are read about every 0.5 s
