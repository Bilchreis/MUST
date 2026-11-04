# Spec Delta

## Purpose

Lets a module work with Rust values instead of SECoP `Value`s, keep its parameters in typed fields that publish updates when set, and be built from settings with configured initial values. This is the runtime layer under the module declaration macro.

## ADDED Requirements

### Requirement: Rust values of SECoP datainfos
The library SHALL convert between Rust values and SECoP values for `f64`, the integer types, `bool`, `String`, `Vec<u8>` (blob), `Vec<T>` (array), tuples, `Status`, and types deriving the enum and struct derives. A value arriving in a request SHALL be checked against the datainfo with all limits before a method receives it.

#### Scenario: Value within limits reaches the method
- **WHEN** a client sends `change T:target 2000` for a target declared `double(max = 1500.0)`
- **THEN** the client receives a `RangeError` and `write_target` is not called

### Requirement: Values returned by methods
A value returned by a read or write method SHALL be published even if it is outside the datainfo's limits, as a node may report a measured value out of range. A returned value that does not fit the datainfo's type, e.g. an array longer than `maxlen`, SHALL be published as an `InternalError` naming the parameter.

#### Scenario: Measured value out of range
- **WHEN** `read_value` returns 1600.0 for a value declared `double(max = 1500.0)`
- **THEN** the reply carries 1600.0

#### Scenario: Array too long
- **WHEN** `read_curve` returns 20 elements for a parameter declared `array(double(), maxlen = 10)`
- **THEN** the reply is an `InternalError` naming `curve`

### Requirement: Enum and struct types
The library SHALL provide a derive for a field-less Rust enum with explicit or implicit discriminants, giving a SECoP `enum` datainfo with the variant names and values. It SHALL provide a derive for a struct with named fields, giving a SECoP `struct` datainfo with one member per field in field order. A member's datainfo SHALL follow from its type unless given by a datainfo attribute on the field, and an `Option` field SHALL be an optional member.

#### Scenario: Enum datainfo
- **WHEN** `enum Mode { Ramp = 1, Pid = 2 }` derives the enum derive and is used as `enum(Mode)`
- **THEN** the datainfo is `{"type": "enum", "members": {"Ramp": 1, "Pid": 2}}`

#### Scenario: Optional struct member
- **WHEN** a struct deriving the struct derive has `p: f64` and `i: Option<f64>`, and a client changes it to `{"p": 40}`
- **THEN** the method receives `p` 40.0 and `i` `None`

### Requirement: Typed parameter values
Each declared parameter SHALL hold its latest value: the last value read, written, set, pushed or configured. Before any value, it SHALL hold its `default`, or else the default of its Rust type. A module method SHALL be able to get it and to set it. Setting it SHALL publish the value as an update to activated clients and store it in the cache.

#### Scenario: Latest read value
- **WHEN** `read_value` returned 4.2 at the last poll
- **THEN** `self.value.get()` returns 4.2 inside the next method call

#### Scenario: Status set while writing the target
- **WHEN** `write_target` sets `status` to BUSY and returns
- **THEN** activated clients receive the `status` update before the `changed` reply

### Requirement: Updates from background tasks
A module SHALL be able to obtain a cloneable handle for a parameter that background tasks can use to push values or errors. Values pushed through a handle SHALL be published at once, and SHALL be returned by the parameter's `get` afterwards.

#### Scenario: Simulation task pushes the value
- **WHEN** a task spawned by the module pushes 300.5 through the `value` handle
- **THEN** activated clients receive an update of `value` with 300.5, and `self.value.get()` returns 300.5 in the next call

### Requirement: One request at a time
A typed module SHALL handle one request (read, change, do, poll) at a time and SHALL give its methods exclusive access to the module (`&mut self`). Requests arriving meanwhile SHALL wait in the order the module receives them.

#### Scenario: Overlapping requests
- **WHEN** two clients send `change T:target 10` and `read T:value` at the same time
- **THEN** `write_target` and `read_value` never run at the same time

### Requirement: Start and poll hooks
A typed module SHALL be able to provide a start hook, run once when the node starts before any other method of the module, and a poll hook, which receives the names of all parameters due at one poll tick. Without a poll hook, the due parameters SHALL be read one after another with their read methods. A failing start hook SHALL be logged with the module name. The module SHALL stay in the node and keep handling requests.

#### Scenario: Connection opened in the start hook
- **WHEN** a module's start hook opens its hardware connection
- **THEN** no `read_` method of the module runs before the start hook has returned

#### Scenario: Start hook fails
- **WHEN** a module's start hook returns an error
- **THEN** the error is logged with the module's name, and the node keeps serving its other modules and this one

### Requirement: Construction from settings
A typed module SHALL be constructed from a module name, a description and a set of settings, keyed by name. Each property SHALL take its setting, or its default if it has none. A property without a setting or default SHALL be an error. A setting that names a parameter SHALL be its configured initial value, checked against its datainfo with all limits. A setting that names neither a property nor a parameter SHALL be an error. Every error SHALL name the module and the setting.

#### Scenario: Missing property
- **WHEN** a module with `heater_range: property!("heater range", int(min = 0, max = 5))` is constructed without `heater_range`
- **THEN** construction fails with an error naming the module and `heater_range`

#### Scenario: Unknown setting
- **WHEN** a module is constructed with the setting `chanel`
- **THEN** construction fails with an error naming `chanel`

### Requirement: Configured initial values
A configured value of a cached parameter SHALL be its value when the node starts. A configured value of a writable parameter that is not cached SHALL be written with `write_<p>` when the node starts, after the start hook and before the startup reads. A configured value of a readonly parameter that is not cached, or of a constant parameter, SHALL be a construction error saying that the parameter cannot be configured.

#### Scenario: Target written at start
- **WHEN** a `Drivable` module is constructed with the setting `target` 300.0 and the node starts
- **THEN** `write_target(300.0)` is called once, after the start hook and before the first `read_value`

#### Scenario: Configured readonly parameter
- **WHEN** a module is constructed with the setting `value` 4.2
- **THEN** construction fails with an error saying that `value` cannot be configured

### Requirement: Failing initial writes
A failing initial write SHALL be logged with the module and parameter name and published as an error update of that parameter. The node SHALL keep running and the module SHALL keep handling requests.

#### Scenario: Initial write fails
- **WHEN** `write_target` fails with `CommunicationFailed` during the initial write
- **THEN** the failure is logged, activated clients see an error update of `target`, and the node keeps serving

### Requirement: Typed modules in a node
A constructed typed module SHALL be added to a node like any other module. Its parameters, commands and properties SHALL come from its declaration, and its requests SHALL go to its methods. The node's completion checks SHALL apply unchanged.

#### Scenario: Typed and hand-written modules together
- **WHEN** a node has one typed module and one module declared with the builder API
- **THEN** both are listed in `describe` and both answer requests
