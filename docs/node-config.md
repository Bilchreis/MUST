# Node config files

A SEC node can be declared in a TOML file instead of in code: node properties, and one
table per module naming its module type and settings. `must serve` runs it:

```bash
must serve crates/must-config/examples/gas-rack.toml
must check crates/must-config/examples/gas-rack.toml   # only check it, list the modules
must types                                             # the module types a config can use
```

This follows Frappy's split between module classes and config files
([issue #59](https://github.com/Bilchreis/MUST/issues/59)), with static configs: a config
file is data, not a program.

## The format

```toml
[node]
equipment_id = "cryo-1"
description = "a simulated cryostat"
interface = "tcp://2055"

[modules.T]
type = "must::sim::Temperature"
description = "sample temperature"
temperature = 290.0     # a property of the type
ramp = 10.0             # a parameter: its initial value
pollinterval = 2.0
```

The `[node]` table:

| Key | Meaning |
|---|---|
| `equipment_id`, `description` | required node properties |
| `firmware` | default `must-<version>` |
| `implementor`, `timeout` | node properties |
| `_name` | custom node properties |
| `interface` | `tcp://<port>` (all interfaces) or `tcp://<host>:<port>`; default port 2055 |
| `discovery` | `false`, or `{ port = ..., announce = ... }` |
| `log_files` | `false`, or `{ dir = "...", days = ... }`; a relative `dir` is relative to the config file |

A `[modules.<name>]` table declares one module, published in file order:

| Key | Meaning |
|---|---|
| `type` | the registered module type, e.g. `must::sim::FlowController` |
| `description` | the module's description; the type's default if omitted |
| `group`, `visibility` | module properties |
| `_name` | custom module properties, e.g. `_serial_number = "M1201"` |
| anything else | the type's settings: its properties, and initial values of its parameters |

**Initial values.** A setting that names a parameter of a macro-declared type sets its
initial value, checked against its datainfo with all limits:

- a writable parameter is written to the hardware when the node starts (`target = 300`
  goes through `write_target`); without a write method it is just stored;
- a readonly parameter without a read method starts with the value;
- a parameter read from the hardware (`value = 4.2`) cannot be configured.

## Many modules of one type: templates

A module whose key contains a placeholder is a template, expanding into one module per
instance:

```toml
# five identical flow controllers: mfc1 ... mfc5
[modules."mfc{i}"]
type = "must::sim::FlowController"
count = 5
range = 100.0
address = "{i}"                  # exactly one placeholder: the integer 1 ... 5
description = "flow controller {i}"

# one instance per table, with what differs
[modules."mfc_{gas}"]
type = "must::sim::FlowController"
description = "{gas} flow"
_gas = "{gas}"
pollinterval = 0.5
instances = [
    { gas = "N2", range = 500.0, _serial_number = "M1201" },
    { gas = "Ar", range = 200.0, _serial_number = "M1202" },
]
```

- `count = N` makes N instances, `instances = [...]` one per table. A template has
  exactly one of the two.
- `{i}` is the instance number, from `first` (default 1).
- The keys of an instance table replace the template's keys of the same name, and are
  placeholders: `{gas}`.
- Placeholders are replaced in the module name and in every string setting of the
  template, nested ones included, but not in the instance tables.
- A string that is exactly one placeholder takes the value's type: `address = "{i}"`
  gives an integer, `range = "{max}"` the instance's `max`. Inside a longer string the
  value's text is inserted.
- `{{` and `}}` stand for literal braces.
- An instance key used only as a placeholder (`gas` above) is not a setting of the type.

Substitution is all there is: no expressions, conditions or loops. Irregular values go
in the instance tables.

## Errors

The whole file is checked before anything is served, and every problem is reported at
once, with the line and column of the offending key or value, the module, and for an
expanded module the template and instance:

```text
rack.toml:7:58: module `mfc3` (template `mfc{i}`, instance 3): `range`: -3 is below the lower limit 0
rack.toml:12:9: module `b`: unknown setting `rnage`: neither a property nor a parameter of the module type `must::sim::FlowController`
```

Errors include unknown types (with the list of registered ones), misspelled settings,
wrong value types, values outside a datainfo's limits, unknown placeholders, and invalid
or duplicate module names. Completion warnings of the node are printed but do not stop
it. `must check` exits with 0 for a valid config, 1 for errors; `must serve` also exits
with 1 if the node cannot listen on its address.

## Module types

`must` knows the built-in simulation types, so configs can be tried without hardware:

- `must::sim::Temperature`: a temperature ramping towards `target` at `ramp` K/min,
  property `temperature` (at start).
- `must::sim::FlowController`: a flow following `target` within `settling_time`
  (default 2 s), properties `range` (required: `target`'s upper limit) and `unit`
  (default `ln/min`).

Hardware drivers are module types declared with `#[must_server::module]` (see
[Writing modules](writing-modules.md)): its `property!` fields and parameters are the
config settings. For modules whose shape depends on the config, e.g. one parameter per
channel, a factory reads its settings by hand:

```rust
registry.register_fn("acme::MultiChannel", |cfg| {
    let channels: u32 = cfg.get("channels")?;
    let mut module = cfg.new_module("multi-channel readout").interface_classes(["Readable"]);
    // ... one parameter per channel ...
    Ok(module)
})?;
```

A driver collection makes its own binary with its types and the built-in ones; it has
the same `serve`, `check` and `types` commands as `must`
(`crates/must-config/examples/acme_node.rs`):

```rust
fn main() -> std::process::ExitCode {
    let mut registry = must_config::Registry::default();
    registry.register::<Valve>("acme::Valve").unwrap();
    must_config::cli::main(registry)
}
```

`must serve` logs to the terminal (`RUST_LOG`, else `info`) and to the monitoring
endpoint, so `must monitor` shows the node and its log.

## Not (yet) supported

- Configs that run code, as Frappy's Python cfg files do.
- Module types that expand at startup through hardware discovery, e.g. one module per
  flow controller found on a bus. The template format leaves room for them.
- Shared IO modules (`io = "bus"`).
- Config files for C and LabVIEW nodes: the SHALL C interface stays code-based.
- Reading Frappy cfg files.
