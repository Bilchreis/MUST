# Spec Delta

## Purpose

Covers the type-level rules for SECoP 2.0 predefined parameter postfixes (`_min`, `_max`, `_limits`, `_enable`): recognizing postfixed names and deciding which datainfo such a parameter must have.

## ADDED Requirements

### Requirement: Recognizing postfixed names
The library SHALL split a parameter name ending in one of the predefined postfixes `_min`, `_max`, `_limits` or `_enable` into its parent name and postfix, provided the parent name is a non-empty identifier (custom parent names included); other names have no postfix.

#### Scenario: Limits postfix
- **WHEN** the name `target_limits` is split
- **THEN** the parent is `target` and the postfix is `_limits`

#### Scenario: Postfix on a custom parameter
- **WHEN** the name `_ramp_max` is split
- **THEN** the parent is `_ramp` and the postfix is `_max`

#### Scenario: Empty parent
- **WHEN** the name `_max` is split
- **THEN** it has no postfix and is an ordinary custom name

#### Scenario: No predefined postfix
- **WHEN** the name `target_maximum` is split
- **THEN** it has no postfix

### Requirement: Dynamic limits need a scalar numeric parent
The library SHALL allow the postfixes `_min`, `_max` and `_limits` only for a parent whose datainfo is `double`, `scaled` or `int`.

#### Scenario: Limits for a double
- **WHEN** `_limits` is checked against a parent datainfo `{"type": "double", "min": 0, "max": 300}`
- **THEN** it is allowed

#### Scenario: Limits for a string
- **WHEN** `_max` is checked against a parent datainfo `{"type": "string"}`
- **THEN** it is rejected with an error stating that dynamic limits need a scalar numeric parameter

### Requirement: Expected postfix datainfo
The library SHALL derive the datainfo a postfixed parameter must have from its parent: the parent's datainfo for `_min` and `_max`, a tuple of two members each equal to the parent's datainfo for `_limits`, and `{"type": "bool"}` for `_enable`.

#### Scenario: Derived limits datainfo
- **WHEN** the expected datainfo for `_limits` is derived from `{"type": "int", "min": 0, "max": 10}`
- **THEN** it is `{"type": "tuple", "members": [{"type": "int", "min": 0, "max": 10}, {"type": "int", "min": 0, "max": 10}]}`

#### Scenario: Enable for any parent
- **WHEN** the expected datainfo for `_enable` is derived from a parent of type `string`
- **THEN** it is `{"type": "bool"}`

### Requirement: Declared postfix datainfo must match
The library SHALL accept a declared datainfo for a postfixed parameter only if it matches the expected datainfo, where two datainfos match when they have the same type and the same SECoP-defined data properties (unknown and custom properties are ignored).

#### Scenario: Matching limits
- **WHEN** a `target_limits` datainfo of a tuple with two `{"type": "double", "min": 0, "max": 300, "unit": "K"}` members is checked against the parent `{"type": "double", "min": 0, "max": 300, "unit": "K"}`
- **THEN** it is accepted

#### Scenario: Mismatching member type
- **WHEN** a `target_limits` datainfo with `int` members is checked against a `double` parent
- **THEN** it is rejected with an error naming the mismatching tuple member

#### Scenario: Mismatching unit
- **WHEN** a `target_max` datainfo `{"type": "double", "unit": "mK"}` is checked against the parent `{"type": "double", "unit": "K"}`
- **THEN** it is rejected
