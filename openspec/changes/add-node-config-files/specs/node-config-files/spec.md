# Spec Delta

## Purpose

Lets a SEC node be declared in a static TOML file, with node properties and module instances of registered module types, including many modules of one type through templates, and checks the file with precise error messages before the node is built.

## ADDED Requirements

### Requirement: Config file structure
A node config file SHALL be a UTF-8 TOML document with a required `[node]` table and a `[modules]` table holding one table per module (`[modules.<key>]`). Any other top-level key SHALL be a config error. A config with no modules SHALL be a config error.

#### Scenario: Minimal config
- **WHEN** a config has `[node]` with `equipment_id = "cryo-1"`, `description = "cryostat"` and one `[modules.T]` of type `must::sim::Temperature` with a description
- **THEN** loading succeeds and the node's `describe` lists module `T`

#### Scenario: Unknown top-level table
- **WHEN** a config has a top-level table `[module.T]`
- **THEN** loading fails with an error naming `module` and its line

#### Scenario: Not TOML
- **WHEN** a config file has a TOML syntax error on line 7
- **THEN** loading fails with an error giving the file, line 7 and the parser's message

### Requirement: Node properties in the config
The `[node]` table SHALL take `equipment_id` and `description` (required), `firmware` (default `must-<version>`), `implementor`, `timeout`, and custom properties whose keys start with `_`. These SHALL be published as the SECoP node properties of the same names. Any other key, except `interface`, `discovery` and `log_files`, SHALL be a config error.

#### Scenario: Custom node property
- **WHEN** `[node]` has `_facility = "HZB"`
- **THEN** the node's `describe` has the property `_facility` with value `"HZB"`

#### Scenario: Missing equipment_id
- **WHEN** `[node]` has no `equipment_id`
- **THEN** loading fails with an error naming `equipment_id`

### Requirement: Node interface in the config
The `[node]` key `interface` SHALL set the TCP address the node listens on, as `tcp://<port>` (all interfaces) or `tcp://<host>:<port>`. Without it the node SHALL listen on port 2055 on all interfaces. Any other form SHALL be a config error.

#### Scenario: Port only
- **WHEN** `interface = "tcp://10767"`
- **THEN** the node listens on port 10767 on all interfaces

#### Scenario: Unsupported scheme
- **WHEN** `interface = "serial:///dev/ttyS0"`
- **THEN** loading fails with an error naming `interface`

### Requirement: Discovery and log files in the config
The `[node]` key `discovery` SHALL be `false` (no UDP discovery) or a table with `port` and `announce`. The key `log_files` SHALL be `false` (no log files) or a table with `dir` and `days`. Omitted keys SHALL keep the library defaults, including those taken from the environment.

#### Scenario: Discovery off
- **WHEN** `[node]` has `discovery = false`
- **THEN** the node does not answer UDP discovery requests

#### Scenario: Log file retention
- **WHEN** `[node]` has `log_files = { days = 7 }`
- **THEN** the node writes log files to the default directory and keeps 7 days

### Requirement: Module declaration in the config
A module table SHALL name its module type with `type` and MAY set `description`, `group`, `visibility` and custom properties whose keys start with `_`. `description` SHALL default to the type's default description and SHALL be a config error if neither exists. An unregistered `type` SHALL be a config error listing the registered types.

#### Scenario: Custom module property
- **WHEN** `[modules.mfc1]` has `_serial_number = "M1234"`
- **THEN** module `mfc1` in `describe` has the property `_serial_number` with value `"M1234"`

#### Scenario: Unknown type
- **WHEN** a module has `type = "must::sim::Flowcontroller"`
- **THEN** loading fails with an error naming the type, the module and the line, and listing the registered type names

### Requirement: Module order
The node SHALL publish the modules in the order their tables appear in the config file. A template's modules SHALL appear at the template's position, in instance order.

#### Scenario: File order kept
- **WHEN** the config declares `[modules.p]`, then a template making `mfc1` to `mfc3`, then `[modules.a]`
- **THEN** `describe` lists the modules as `p`, `mfc1`, `mfc2`, `mfc3`, `a`

### Requirement: Parameter initial values in the config
A module setting that the type does not read and that names a cached parameter of the module, including a `pollinterval` parameter, SHALL set that parameter's initial value. The value SHALL be checked against the parameter's datainfo with all limits. A setting that names a parameter that is not cached SHALL be a config error saying that the parameter is read from the hardware.

#### Scenario: Poll interval from the config
- **WHEN** a module of type `must::sim::Temperature` has `pollinterval = 2.0`
- **THEN** a `read` of its `pollinterval` returns 2.0 and `value` is polled every 2 s

#### Scenario: Initial value out of range
- **WHEN** a module sets `pollinterval = -1.0` and the parameter's datainfo has `min` 0
- **THEN** loading fails with an error naming the module, `pollinterval` and the line, with the range violation

#### Scenario: Value of a hardware parameter
- **WHEN** a module of type `must::sim::Temperature` sets `value = 4.2`
- **THEN** loading fails with an error saying that `value` is read from the hardware and cannot be set in the config

### Requirement: Unknown module settings
A module setting that is not one of the keys of the module declaration, is not read by the type, does not name a cached parameter and is not a template variable SHALL be a config error. The error SHALL name the module, the type and the key, with its location.

#### Scenario: Misspelled setting
- **WHEN** a `must::sim::FlowController` module has `rnage = 200.0` next to `range = 200.0`
- **THEN** loading fails with an error naming `rnage`, the module, the type and the line

### Requirement: Module templates
A module table whose key contains a placeholder (`{name}`) SHALL be a template that expands into several modules of its `type`. It SHALL have exactly one of `count` (an integer of at least 1) or `instances` (a non-empty array of tables). `first` (integer, default 1) SHALL set the number of the first instance. `count`, `instances` and `first` on a table without a placeholder SHALL be config errors.

#### Scenario: Five identical flow controllers
- **WHEN** a config has `[modules."mfc{i}"]` with `type = "must::sim::FlowController"`, `count = 5` and `range = 100.0`
- **THEN** the node has modules `mfc1` to `mfc5`, each with a `target` limit of 100.0

#### Scenario: Numbering from another start
- **WHEN** a template `"valve{i}"` has `count = 3` and `first = 0`
- **THEN** the node has modules `valve0`, `valve1` and `valve2`

#### Scenario: Template without count or instances
- **WHEN** a table `[modules."mfc{i}"]` has neither `count` nor `instances`
- **THEN** loading fails with an error naming the template

#### Scenario: Both count and instances
- **WHEN** a template has `count = 2` and `instances = [{}, {}]`
- **THEN** loading fails with an error naming the template

### Requirement: Template instances
Each table in a template's `instances` SHALL describe one module. Its keys SHALL override the template's settings of the same name, and SHALL be available as placeholders. An instance SHALL NOT set `type`, `count`, `instances` or `first`. An instance key that is used as a placeholder and is nothing else to the module SHALL be accepted as a template variable.

#### Scenario: Per-instance settings
- **WHEN** a template `"mfc_{gas}"` with `range = 100.0` has `instances = [{ gas = "N2", range = 500.0 }, { gas = "Ar" }]`
- **THEN** the node has `mfc_N2` with a `target` limit of 500.0 and `mfc_Ar` with 100.0

#### Scenario: Instance changes the type
- **WHEN** an instance table has `type = "must::sim::Temperature"`
- **THEN** loading fails with an error naming the template, the instance and `type`

### Requirement: Placeholder substitution
In a template, `{i}` (the instance number) and `{<key>}` for each key of the current instance table SHALL be replaced in the module name and in every string setting, including strings nested in arrays and tables. `{{` and `}}` SHALL stand for literal braces. A placeholder that is not defined for an instance SHALL be a config error. Values in instance tables SHALL NOT be substituted.

#### Scenario: Description from instance values
- **WHEN** a template has `description = "{gas} flow, controller {i}"` and its second instance has `gas = "Ar"`
- **THEN** that module's description is `Ar flow, controller 2`

#### Scenario: Unknown placeholder
- **WHEN** a template `"mfc{n}"` has `count = 2`
- **THEN** loading fails with an error naming the placeholder `n` and the template

#### Scenario: Literal braces
- **WHEN** a template sets `_fmt = "{{value}}"`
- **THEN** each module's `_fmt` property is `{value}`

### Requirement: Typed placeholders
A string setting that consists of exactly one placeholder SHALL be replaced by the variable's value with its own TOML type: an integer for `{i}`, and the instance value's type for `{<key>}`. A placeholder inside a longer string SHALL insert the value's text: an integer or float as written in decimal, a boolean as `true` or `false`, a string unchanged. Inserting an array or table into a longer string SHALL be a config error.

#### Scenario: Bus address from the instance number
- **WHEN** a template with `count = 3` and `first = 4` has `address = "{i}"`
- **THEN** the type of the three modules reads the integer settings 4, 5 and 6 for `address`

#### Scenario: Range from an instance value
- **WHEN** a template has `range = "{max}"` and an instance has `max = 50.0`
- **THEN** the type reads the float 50.0 for `range` of that instance

### Requirement: Expanded module names
Every module name, whether declared directly or expanded from a template, SHALL be a valid SECoP identifier and unique in the node, ignoring case. A violation SHALL be a config error naming both declarations, with the template and instance for an expanded name.

#### Scenario: Template collides with a module
- **WHEN** a config has `[modules.mfc2]` and a template `"mfc{i}"` with `count = 3`
- **THEN** loading fails with an error naming `mfc2`, the module table and the template's second instance

#### Scenario: Invalid expanded name
- **WHEN** a template `"mfc_{gas}"` has an instance with `gas = "N2 low"`
- **THEN** loading fails with an error saying that `mfc_N2 low` is not a valid SECoP identifier

### Requirement: Config error reporting
Loading SHALL check the whole config before building the node and SHALL report every error found, not only the first. Each error SHALL give the file path, the line and column of the offending key or value, the module name, and for an expanded module the template key and instance number. Completion warnings of the built node SHALL be reported with the module names, and SHALL NOT make loading fail.

#### Scenario: Several errors at once
- **WHEN** a config has an unknown type in one module and a misspelled setting in another
- **THEN** loading fails with both errors, each with its own line

#### Scenario: Error in an expanded module
- **WHEN** the third instance of template `"mfc{i}"` is rejected by its type
- **THEN** the error names module `mfc3`, the template `mfc{i}` and instance 3, with the line of the offending value

#### Scenario: Warnings do not fail loading
- **WHEN** a config's node builds with a completion warning
- **THEN** loading succeeds and the warning is reported with its module name
