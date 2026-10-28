# Spec Delta

## ADDED Requirements

### Requirement: Poll configuration
The library SHALL let an application configure polling per module: keep the default tiers (fast 5 s, slow 60 s), set the fast interval, set the slow interval, set a uniform interval for all parameters instead of tiers, or turn module-level polling off; and per parameter, an interval of its own. Setting more than one of fast interval, uniform interval and polling off on one module SHALL be rejected. Intervals SHALL be positive. A parameter interval on a constant or cached parameter, or on the `pollinterval` parameter, SHALL be ignored and reported as a completion warning. A slow interval on a module with a uniform interval or with polling off SHALL be reported as a completion warning.

#### Scenario: Default tiers
- **WHEN** a `Drivable` module with `value`, `status`, `target` and `ramp` sets no poll configuration
- **THEN** completion succeeds, `value` and `status` are in the fast tier at 5 s, and `target` and `ramp` in the slow tier at 60 s

#### Scenario: Tiered and uniform together
- **WHEN** a module is given both a fast interval of 1 s and a uniform interval of 2 s
- **THEN** completion fails with an error naming the module

#### Scenario: Polling off with a uniform interval
- **WHEN** a module has polling turned off and a uniform interval of 2 s
- **THEN** completion fails with an error naming the module

#### Scenario: Interval on a cached parameter
- **WHEN** a cached parameter `target_max` is given a poll interval
- **THEN** completion succeeds with a warning naming `target_max`, and it is never polled

#### Scenario: Tiers of a module without a base class
- **WHEN** a module with `interface_classes` `[]` and parameters `value` and `x` keeps the default tiers
- **THEN** completion succeeds and both parameters are in the slow tier

### Requirement: Poll interval parameter declaration
The library SHALL treat a module parameter named `pollinterval` as the module's poll interval parameter (see `sec-node-runtime`) when its datainfo is a `double` without a negative `min`. A `pollinterval` parameter with any other datainfo SHALL be accepted as an ordinary parameter and reported as a completion warning. On a module with polling turned off, it SHALL be reported as a completion warning, since it has no effect. The poll interval parameter may be readonly or writable, and is published in the structure report like any other parameter.

#### Scenario: Valid pollinterval
- **WHEN** a module declares a writable `pollinterval` with datainfo `{"type": "double", "min": 0, "unit": "s"}`
- **THEN** completion succeeds without a warning about it

#### Scenario: Integer pollinterval
- **WHEN** a module declares `pollinterval` with datainfo `{"type": "int"}`
- **THEN** completion succeeds with a warning that it does not drive polling, and it is read through the handler like any other parameter
