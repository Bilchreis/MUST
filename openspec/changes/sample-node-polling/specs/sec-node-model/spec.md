# Spec Delta

## ADDED Requirements

### Requirement: Poll configuration
The library SHALL let an application configure polling per module, either as a tiered interval (fast tier at the given interval, slow tier at 60 s or a configured slow interval) or as a uniform interval, and per parameter as an interval of its own. Configuring both a tiered and a uniform interval on one module SHALL be rejected. Intervals SHALL be positive. A parameter interval on a constant or cached parameter, or on the `pollinterval` parameter, SHALL be ignored and reported as a completion warning. Without any configuration, nothing is polled.

#### Scenario: Tiered and uniform together
- **WHEN** a module is given both a tiered interval of 1 s and a uniform interval of 2 s
- **THEN** completion fails with an error naming the module

#### Scenario: Interval on a cached parameter
- **WHEN** a cached parameter `target_max` is given a poll interval
- **THEN** completion succeeds with a warning naming `target_max`, and it is never polled

#### Scenario: Tiers of a module without a base class
- **WHEN** a module with `interface_classes` `[]` and parameters `value` and `x` gets a tiered interval
- **THEN** completion succeeds and both parameters are in the slow tier

### Requirement: Poll interval parameter declaration
The library SHALL treat a module parameter named `pollinterval` as the module's poll interval parameter (see `sec-node-runtime`) when its datainfo is a `double` without a negative `min`. A `pollinterval` parameter with any other datainfo SHALL be accepted as an ordinary parameter and reported as a completion warning. The poll interval parameter may be readonly or writable, and is published in the structure report like any other parameter.

#### Scenario: Valid pollinterval
- **WHEN** a module declares a writable `pollinterval` with datainfo `{"type": "double", "min": 0, "unit": "s"}`
- **THEN** completion succeeds without a warning about it

#### Scenario: Integer pollinterval
- **WHEN** a module declares `pollinterval` with datainfo `{"type": "int"}`
- **THEN** completion succeeds with a warning that it does not drive polling, and it is read through the handler like any other parameter
