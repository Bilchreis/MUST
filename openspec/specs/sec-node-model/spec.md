# sec-node-model Specification

## Purpose

Lets an application declare a SEC node (its modules, parameters, commands and properties), checks the declaration against the SECoP 2.0 rules before the node goes online, and produces the structure report sent in reply to `describe`.

## Requirements

### Requirement: Node declaration
The library SHALL let an application declare a node with the mandatory properties `equipment_id` and `description`, the optional properties `firmware`, `implementor`, `timeout`, `systems` and `schemata`, and custom properties whose names start with an underscore.

#### Scenario: Minimal node
- **WHEN** a node with equipment_id `HZB-vm4`, description `test node` and one valid module is declared and completed
- **THEN** completion succeeds

#### Scenario: Missing description
- **WHEN** a node with an empty description is completed
- **THEN** completion fails with an error naming the missing node property `description`

### Requirement: Module declaration
The library SHALL let an application add modules with the mandatory properties `description` and `interface_classes`, the optional properties `features`, `visibility`, `group`, `meaning`, `implementor` and `implementation`, and custom properties.

#### Scenario: Module without description
- **WHEN** a module without a description is added and the node is completed
- **THEN** completion fails with an error naming the module and the missing property

### Requirement: Accessible declaration
The library SHALL let an application add parameters (with `description`, `datainfo`, `readonly` and the optional `group`, `visibility`, `meaning`, `checkable`, `constant`) and commands (with `description`, a `command` datainfo and the same optional properties except `constant`), plus custom properties.

#### Scenario: Command datainfo for a parameter
- **WHEN** a parameter is declared with datainfo `{"type": "command"}`
- **THEN** the declaration is rejected

#### Scenario: Constant value must fit the datainfo
- **WHEN** a parameter with datainfo `{"type": "int", "min": 0, "max": 10}` is declared with `constant` 20
- **THEN** the declaration is rejected with a `RangeError` description

### Requirement: Unknown property names
The library SHALL reject a property whose name is neither SECoP-defined nor starts with an underscore, unless the application declares it as a non-standard property, in which case it is published unchanged and reported as a warning at completion.

#### Scenario: Unknown property rejected
- **WHEN** a module property `myprop` is added normally
- **THEN** the addition fails with an error suggesting `_myprop`

#### Scenario: Non-standard property allowed explicitly
- **WHEN** a module property `myprop` is added as non-standard
- **THEN** completion succeeds with a warning and the structure report contains `myprop`

### Requirement: Names and uniqueness
The library SHALL require module, accessible and property names to be valid identifiers, unique case-insensitively within their scope, and SHALL reject a `group` path component that matches a module name, or (for accessible groups) a module or accessible name, case-insensitively.

#### Scenario: Module names clash
- **WHEN** modules `T1` and `t1` are added to one node
- **THEN** adding `t1` fails

#### Scenario: Group clashes with a module
- **WHEN** a node has modules `cryo` and `t1`, and `t1` has `group` `Cryo:sensors`
- **THEN** completion fails with an error naming the group component `Cryo`

### Requirement: Property values
The library SHALL check SECoP-defined property values: `visibility` must be one of the new-style or old-style values listed by the spec, `interface_classes` and `features` must be lists of strings, `timeout` a positive number, and `meaning` an object.

#### Scenario: Invalid visibility
- **WHEN** a parameter has `visibility` `"wxw"`
- **THEN** completion fails with an error naming the visibility value

#### Scenario: Old-style visibility
- **WHEN** a module has `visibility` `"expert"`
- **THEN** it is accepted and reported unchanged in the structure report

### Requirement: Interface class names
The library SHALL accept in `interface_classes` only the SECoP 2.0 interface classes (`Readable`, `Writable`, `Drivable`, `Communicator`, `AcquisitionController`, `AcquisitionChannel`, `Acquisition`) and custom names starting with an underscore.

#### Scenario: Custom class before a base class
- **WHEN** a module has `interface_classes` `["_MyHeater", "Readable"]` and parameters `value` and `status`
- **THEN** completion succeeds

#### Scenario: Non-standard class without underscore
- **WHEN** a module has `interface_classes` `["MyHeater", "Readable"]`
- **THEN** completion fails with an error naming `MyHeater` and suggesting `_MyHeater`

### Requirement: Interface class rules
The library SHALL check the module's base class (the last entry of `interface_classes`): `Readable` needs parameters `value` and `status`; `Writable` additionally a writable `target`; `Drivable` additionally a `stop` command.

#### Scenario: Drivable without stop
- **WHEN** a module with `interface_classes` `["Drivable"]` has `value`, `status` and `target` but no `stop`
- **THEN** completion fails with an error naming the missing command `stop`

### Requirement: Default module properties
The library SHALL publish the module properties `features` and `implementation` even when the application does not set them, defaulting to an empty list and to the Rust type name of the module's handler, and SHALL publish the application's values instead when it sets them.

#### Scenario: Defaults published
- **WHEN** a module with handler type `simple_node::Handler` sets neither `features` nor `implementation`
- **THEN** the structure report contains `"features": []` and `"implementation": "simple_node::Handler"` for it

#### Scenario: Application values win
- **WHEN** a module sets `features` to `["HasOffset"]` and `implementation` to `"lakeshore.Model336"`
- **THEN** the structure report contains exactly those values

### Requirement: Status parameter shape
The library SHALL require a `status` parameter to be a tuple of an enum (all member values between 0 and 499) and a string.

#### Scenario: Valid status
- **WHEN** `status` has datainfo `{"type": "tuple", "members": [{"type": "enum", "members": {"IDLE": 100, "BUSY": 300, "ERROR": 400}}, {"type": "string"}]}`
- **THEN** completion succeeds

#### Scenario: Status code out of range
- **WHEN** the status enum contains a member with value 500
- **THEN** completion fails

### Requirement: Module-level postfix rules
The library SHALL require a postfixed parameter (`_min`, `_max`, `_limits`, `_enable`) to have its parent parameter in the same module with a matching datainfo, and SHALL reject a module that has both `<p>_limits` and `<p>_min` or `<p>_max` for the same parent.

#### Scenario: Limits and max together
- **WHEN** a module has parameters `target`, `target_limits` and `target_max`
- **THEN** completion fails with an error naming both postfixed parameters

#### Scenario: Postfix without parent
- **WHEN** a module has `ramp_max` but no `ramp` parameter
- **THEN** `ramp_max` is treated as an ordinary parameter and completion succeeds

### Requirement: Completion freezes the structure
The library SHALL check the whole declaration when the node is completed, report all problems found (not only the first), and SHALL not allow structural changes after successful completion.

#### Scenario: Several problems at once
- **WHEN** a node with two invalid modules is completed
- **THEN** the error lists the problems of both modules

#### Scenario: Change after completion
- **WHEN** an application tries to add a module to a completed node
- **THEN** the call fails and the node is unchanged

### Requirement: Structure report
The library SHALL produce the SECoP 2.0 structure report as a JSON object with the node properties, a `modules` object and per module an `accessibles` object, keeping declaration order and including custom properties.

#### Scenario: Describe output
- **WHEN** the structure report of a node with module `t1` and parameter `value` is produced
- **THEN** it contains `modules.t1.accessibles.value.datainfo` equal to the declared datainfo and `modules.t1.accessibles.value.readonly` true

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
