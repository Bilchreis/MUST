# Spec Delta

## Purpose

Lets a driver declare a SECoP module as one Rust struct, with one line per parameter, command or config property and datainfos in SECoP's own names, and implement its behaviour as `read_`/`write_`/`do_` methods, with mistakes caught at compile time.

## ADDED Requirements

### Requirement: Module declaration attribute
The library SHALL provide an attribute for a struct with named fields that declares it a module type. Its arguments SHALL be an optional interface class (`Readable`, `Writable` or `Drivable`), which sets `interface_classes`, and an optional default `description`. Without an interface class the module SHALL have empty `interface_classes`. Applying the attribute to anything else SHALL be a compile error.

#### Scenario: Drivable module
- **WHEN** a struct is declared with interface class `Drivable` and served as module `T`
- **THEN** `describe` lists `T` with `interface_classes` `["Drivable"]`

#### Scenario: Attribute on an enum
- **WHEN** the attribute is applied to an enum
- **THEN** compilation fails with an error at the attribute

### Requirement: Parameter declarations
A field of the form `name: param!(description, datainfo, flags…)` SHALL declare a parameter named after the field, in field order. The flags SHALL be `writable`, `cached`, `constant = expr`, `default = expr`, `group = expr`, `visibility = expr`, `checkable`, `poll_interval = expr` and custom properties `_name = expr`. An unknown flag, a repeated flag, or `constant` together with `writable`, `cached` or `default` SHALL be a compile error at the flag.

#### Scenario: Declared parameter in describe
- **WHEN** a module declares `target: param!("setpoint", double(min = 0.0, max = 1500.0, unit = "K"), writable)`
- **THEN** `describe` lists `target` with description `setpoint`, `readonly` false and datainfo `{"type": "double", "min": 0.0, "max": 1500.0, "unit": "K"}`

#### Scenario: Unknown flag
- **WHEN** a parameter is declared with the flag `writeable`
- **THEN** compilation fails with an error at `writeable`

### Requirement: Datainfo syntax
A datainfo SHALL be written as a SECoP datainfo type name with SECoP's keys as named arguments, whose values are Rust expressions: `double`, `scaled`, `int`, `bool`, `string`, `blob`, `array(elem, …)`, `tuple(a, b, …)`, `enum(Type)`, `struct(Type)`, and `status(codes…)` for a status tuple with the named SECoP status codes. A key that the datainfo type does not have SHALL be a compile error at the key. `datainfo(expr)` SHALL accept a datainfo built in code, together with `as T`.

#### Scenario: Unit is published
- **WHEN** a parameter has datainfo `double(min = 0.0, unit = "K", fmtstr = "%.3f")`
- **THEN** its datainfo in `describe` has `min` 0.0, `unit` `K` and `fmtstr` `%.3f`

#### Scenario: Key of another type
- **WHEN** a parameter has datainfo `double(maxchars = 8)`
- **THEN** compilation fails with an error at `maxchars` saying that `double` has no key `maxchars`

#### Scenario: Status codes
- **WHEN** a parameter has datainfo `status(IDLE, BUSY, ERROR)`
- **THEN** its datainfo is a tuple of an enum with members `IDLE` 100, `BUSY` 300 and `ERROR` 400, and a string

### Requirement: Datainfos from properties
A datainfo value of a parameter or command SHALL be able to name a property of the same module, which stands for that property's value. Such datainfos SHALL be built when the module is constructed, after its properties are set, so that each module instance gets its own datainfos. A datainfo of a property naming another property SHALL be a compile error.

#### Scenario: Limit and unit from properties
- **WHEN** a module declares `range: property!("full scale", double(min = 0.0))`, `unit: property!("unit", string(), default = "ln/min")` and `target: param!("setpoint", double(min = 0.0, max = range, unit = unit), writable)`, and two instances are constructed with `range` 200.0 and 500.0
- **THEN** `describe` shows a `target` `max` of 200.0 for the first and 500.0 for the second, both with unit `ln/min`

#### Scenario: Property naming a property
- **WHEN** a property's datainfo is `double(max = range)` and `range` is another property
- **THEN** compilation fails with an error at `range`

### Requirement: Rust types of declared accessibles
The Rust type of a parameter or property SHALL follow from its datainfo: `double` and `scaled` give `f64`, `int` gives `i64`, `bool` gives `bool`, `string` gives `String`, `blob` gives `Vec<u8>`, `array` a `Vec` of its element type, `tuple` a tuple of its members, `enum(T)` and `struct(T)` give `T`, and `status` gives `Status`.

#### Scenario: Default type of a double
- **WHEN** a parameter is declared `double(unit = "K")`
- **THEN** the struct field holds an `f64`

### Requirement: Selecting the Rust type
`as T` after a datainfo SHALL select another Rust type that maps to the same datainfo type. A type that does not map to it SHALL be a compile error at the type. Limits that do not fit the selected type SHALL be a module construction error naming the accessible and the range.

#### Scenario: Integer as u8
- **WHEN** a property is declared `int(min = 1, max = 2) as u8`
- **THEN** the struct field has type `u8`

#### Scenario: Wrong Rust type
- **WHEN** a parameter is declared `double() as String`
- **THEN** compilation fails with an error at `String`

#### Scenario: Limits too wide
- **WHEN** a parameter is declared `int(min = 0, max = 300) as u8`
- **THEN** constructing the module fails with an error naming the parameter and the range

### Requirement: Command declarations
A field of the form `name: command!(description, flags…)` SHALL declare a command named after the field. The flags SHALL be `argument = datainfo`, `result = datainfo`, `group`, `visibility`, `checkable` and custom properties. Without `argument` or `result` the command SHALL take no argument or return no result.

#### Scenario: Command with argument and result
- **WHEN** a module declares `wait: command!("wait for stability", argument = double(min = 0.0, unit = "s"), result = bool())`
- **THEN** `describe` lists `wait` with a command datainfo with that argument and result

### Requirement: Property declarations
A field of the form `name: property!(description, datainfo, default = expr)` SHALL declare a config-only setting of the module, with `default` optional. A property SHALL NOT be a SECoP accessible. It SHALL be set when the module is constructed, from the module's settings, and its value SHALL be checked against its datainfo with all limits.

#### Scenario: Property is not published
- **WHEN** a module declares `channel: property!("sensor channel", string(maxchars = 1))`
- **THEN** `describe` has no accessible `channel`

#### Scenario: Property out of range
- **WHEN** a module with `heater_loop: property!("heater loop", int(min = 1, max = 2), default = 1)` is constructed with `heater_loop` 3
- **THEN** construction fails with an error naming `heater_loop` and the range

### Requirement: Internal state fields
A field without `param!`, `command!` or `property!` SHALL be the module's own state. It SHALL NOT be published, and SHALL be initialized with its type's default value when the module is constructed.

#### Scenario: State field
- **WHEN** a module has a plain field `driving: bool`
- **THEN** the field is `false` after construction and `describe` has no accessible `driving`

### Requirement: Behaviour methods
The module type SHALL implement async methods taking `&mut self` and returning the type's `Result`: `read_<p>()` for every parameter that is neither cached nor constant, `write_<p>(value)` returning the value actually set for every writable parameter that is not cached, and `do_<c>([argument])` for every command. A missing method, or one with the wrong types, SHALL be a compile error naming the accessible.

#### Scenario: Missing read method
- **WHEN** a module declares `sample: param!("sample temperature", double())` and has no `read_sample` method
- **THEN** compilation fails with an error naming `read_sample`, at the `sample` declaration

### Requirement: Cached parameters without methods
A cached parameter SHALL be served from its stored value without calling a method. A `change` of a writable cached parameter SHALL store the new value and reply with it without calling a method.

#### Scenario: Cached writable parameter
- **WHEN** a module declares `tolerance: param!("convergence criterion", double(min = 0.0), writable, cached, default = 0.1)` and a client sends `change T:tolerance 0.2`
- **THEN** the reply is `changed T:tolerance` with 0.2, a later `read` returns 0.2, and no method of the module is called

### Requirement: Interface class checks
The macro SHALL check the declarations against the interface class at compile time:
- `Readable` needs a readable parameter `value` and a parameter `status` with a status datainfo
- `Writable` additionally needs a writable parameter `target`
- `Drivable` additionally needs a command `stop` without argument and result, and a status that includes `BUSY`

A violation SHALL be a compile error naming the missing or wrong accessible.

#### Scenario: Drivable without stop
- **WHEN** a struct declared `Drivable` has `value`, `status` and a writable `target` but no `stop`
- **THEN** compilation fails with an error saying that `Drivable` needs a command `stop`

#### Scenario: Readonly target
- **WHEN** a struct declared `Writable` declares `target` without `writable`
- **THEN** compilation fails with an error at `target` saying it must be writable

### Requirement: Macros outside a module declaration
`param!`, `command!` and `property!` used anywhere other than as a field type of a struct with the module declaration attribute SHALL be compile errors saying where they belong.

#### Scenario: Stray param macro
- **WHEN** `param!("x", double())` is used as the type of a local variable
- **THEN** compilation fails with an error saying that `param!` is only valid in a module declaration
