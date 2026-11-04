# Spec Delta

## Purpose

Lets driver code declare reusable module types that turn one module's config settings into a complete SECoP module, and collects them in a registry from which node config files pick types by name.

## ADDED Requirements

### Requirement: Module type contract
The library SHALL let an application define a module type that, given one module instance's name, description and settings, returns a complete module (interface classes, parameters, commands, handler) or config errors. A module type SHALL be usable many times in one node, and each use SHALL get its own module and handler.

#### Scenario: Two instances of one type
- **WHEN** a node config declares modules `mfc1` and `mfc2` with the same type
- **THEN** the node has two modules with the type's accessibles, and a `change mfc1:target` reaches only `mfc1`'s handler

#### Scenario: Type rejects a setting
- **WHEN** a module type reports that the setting `range` must be positive and the config gives `range = -1`
- **THEN** loading fails with an error naming the module, the setting `range`, its file location and the type's message

### Requirement: Macro-declared module types
A struct declared with the module declaration macro SHALL be registrable as a module type. Its settings SHALL be its properties and the initial values of its parameters, checked against their datainfos as the typed-module construction rules say. Its declared description SHALL be the default description. Errors SHALL carry the config file location of the offending value.

#### Scenario: Property from the config
- **WHEN** a macro-declared type with `channel: property!("sensor channel", string(maxchars = 1))` is used with `channel = "A"`
- **THEN** the module is built with `channel` `"A"`

#### Scenario: Property value too long
- **WHEN** the same type is used with `channel = "AB"` on line 12
- **THEN** loading fails with an error naming the module and `channel`, at line 12, saying that at most 1 character is allowed

### Requirement: Settings access for module type factories
A module type written as a factory SHALL read its settings by key as typed values (integers, floats, booleans, strings, arrays, tables), required or optional with a default. A missing required setting or a value of the wrong type SHALL be a config error naming the module, the key, the expected type and the location of the value, or of the module if the key is missing.

#### Scenario: Missing required setting
- **WHEN** a factory requires the setting `range` and the module's table has no `range`
- **THEN** loading fails with an error naming the module and the missing setting `range`

#### Scenario: Wrong value type
- **WHEN** a factory reads `range` as a float and the config gives `range = "high"`
- **THEN** loading fails with an error naming `range`, saying a number was expected, with the line of the value

#### Scenario: Integer for a float
- **WHEN** a factory reads `range` as a float and the config gives `range = 500`
- **THEN** the factory receives 500.0

### Requirement: Module type registry
The library SHALL keep module types in a registry under names made of identifiers joined by `::` (e.g. `must::sim::FlowController`). Registering a name twice, or a name of another form, SHALL be rejected. A driver collection SHALL be able to register all its types with one call. A registry SHALL be able to list its type names.

#### Scenario: Duplicate type name
- **WHEN** two types are registered as `acme::Valve`
- **THEN** the second registration is rejected with an error naming `acme::Valve`

#### Scenario: Invalid type name
- **WHEN** a type is registered as `acme:Valve`
- **THEN** the registration is rejected

### Requirement: Built-in simulation types
The library SHALL provide the simulation types `must::sim::Temperature` (a `Drivable` temperature that ramps towards `target`, with `ramp`, `pollinterval` and `stop`) and `must::sim::FlowController` (a `Drivable` flow with `value`, `target`, `status` and `pollinterval`, whose `target` limit is its required `range` setting and whose unit is its optional `unit` setting, default `ln/min`). They SHALL be registered in the default registry and need no hardware.

#### Scenario: Flow controller range
- **WHEN** a `must::sim::FlowController` module `mfc1` with `range = 200.0` is served and a client sends `change mfc1:target 250`
- **THEN** the client receives a `RangeError`

#### Scenario: Simulated flow follows the target
- **WHEN** a client sends `change mfc1:target 100` to a `must::sim::FlowController` module
- **THEN** `mfc1:value` reaches 100 within the simulated settling time and `status` returns to IDLE
