# Writing modules

A module is one Rust struct: one line per parameter, command or config property, with
datainfos in SECoP's own names. Its behaviour lives in `read_<p>`, `write_<p>` and
`do_<c>` methods, as in Frappy. Mistakes in either are compile errors.

```rust
use must_server::typed::prelude::*;

#[module(Drivable, description = "LakeShore temperature loop")]
pub struct TemperatureLoop {
    value: param!("current temperature", double(min = 0.0, max = 1500.0, unit = "K")),
    status: param!("current status", status(IDLE, WARN, BUSY, ERROR)),
    target: param!("setpoint", double(min = 0.0, max = 1500.0, unit = "K"), writable),
    tolerance: param!("convergence criterion", double(min = 0.0, unit = "K"), writable, default = 0.1),
    pollinterval: param!("polling interval", double(min = 0.1, max = 120.0, unit = "s"), writable, default = 5.0),
    stop: command!("stop at the current temperature"),

    uri: property!("controller address", string()),
    channel: property!("sensor channel", string(minchars = 1, maxchars = 1)),
    heater_loop: property!("heater loop", int(min = 1, max = 2) as u8, default = 1),

    conn: Option<Connection>,   // the module's own state
}

#[methods]
impl TemperatureLoop {
    async fn start(&mut self) -> Result<()> {
        self.conn = Some(Connection::open(&self.uri).await?);
        Ok(())
    }

    async fn read_value(&mut self) -> Result<f64> {
        self.query(&format!("KRDG? {}", self.channel)).await?.parse().map_err(comm_failed)
    }

    async fn read_status(&mut self) -> Result<Status> { /* ... */ }

    async fn write_target(&mut self, target: f64) -> Result<f64> {
        self.query(&format!("SETP {},{target};*OPC?", self.heater_loop)).await?;
        self.status.set(Status::busy("target changed"))?;  // reaches clients before `changed`
        Ok(target)                                          // the value actually set
    }

    async fn do_stop(&mut self) -> Result<()> { /* ... */ }
    // tolerance and pollinterval have no methods: reads return them, changes store them
}
```

A module is constructed from its settings and added to a node like any other:

```rust
let t = must_server::typed::module::<TemperatureLoop>(
    "T",
    Some("sample temperature"),
    serde_json::json!({"uri": "tcp://ls336:7777", "channel": "A"}),
)?;
let node = must_server::NodeBuilder::new("cryo-1", "cryostat").module(t)?.build()?;
```

`examples/typed_node.rs` is a complete simulated cryostat written this way; it answers
exactly like `examples/simple_node.rs`, which uses the builder API.

## Declarations

`#[module(...)]` takes an optional interface class (`Readable`, `Writable` or `Drivable`),
which sets `interface_classes`, and `description = "..."`, the default description of
modules of this type. Without an interface class the module has empty
`interface_classes`.

| Field | Declares |
|---|---|
| `name: param!("description", datainfo, flags...)` | a parameter |
| `name: command!("description", flags...)` | a command |
| `name: property!("description", datainfo, default = ...)` | a config-only setting, not an accessible |
| `name: Type` | the module's own state, initialized with `Default` |

Accessibles are published in field order.

**Parameter flags:** `writable`, `default = expr` (the value before any other),
`constant = expr`, `group = "..."`, `visibility = "..."`, `checkable`,
`poll_interval = Duration`, and custom properties `_name = expr`.

**Command flags:** `argument = datainfo`, `result = datainfo`, `group`, `visibility`,
`checkable`, and custom properties. Without `argument` or `result` the command takes no
argument or returns nothing.

**Interface classes** are checked at compile time: `Readable` needs a parameter `value`
and a `status` with a status datainfo, `Writable` also a writable `target`, `Drivable`
also a command `stop` without argument and result, and `BUSY` among its status codes.

## Datainfos

A datainfo is a SECoP type name with SECoP's keys as named arguments. Values are Rust
expressions, so constants work, and so do the module's properties: `double(max = range,
unit = unit)` gives each module its own limits and unit.

| Datainfo | Keys | Rust type |
|---|---|---|
| `double(...)` | `min`, `max`, `unit`, `fmtstr`, `absolute_resolution`, `relative_resolution` | `f64` |
| `scaled(...)` | `scale` and the keys of `double` | `f64` |
| `int(...)` | `min`, `max`, `unit` | `i64` |
| `bool()` | | `bool` |
| `string(...)` | `minchars`, `maxchars`, `isUTF8` | `String` |
| `blob(...)` | `minbytes`, `maxbytes` | `Vec<u8>` |
| `array(elem, ...)` | `minlen`, `maxlen` | `Vec` of the element type |
| `tuple(a, b, ...)` | | a tuple of the member types |
| `enum(T)` | | `T`, with `#[derive(SecopEnum)]` |
| `struct(T)` | | `T`, with `#[derive(SecopStruct)]` |
| `status(IDLE, BUSY, ...)` | | `Status` |
| `datainfo(expr) as T` | | `T`; `expr` is a `DataInfo` built in code |

`as T` picks another Rust type of the same kind: `int(min = 1, max = 2) as u8`. A type
that does not fit the datainfo is a compile error; limits that do not fit it (e.g.
`int(min = 0, max = 300) as u8`) are an error when the module is constructed. An `int`
without `min` and `max` gets the range of its Rust type.

Enums and structs derive their datainfo:

```rust
#[derive(Clone, SecopEnum)]
enum Mode { Ramp = 1, Pid = 2 }

#[derive(Clone, SecopStruct)]
struct Pid {
    #[datainfo(double(min = 0.0))]
    p: f64,
    i: Option<f64>,   // an optional member
}
```

## Behaviour methods

All behaviour methods are `async`, take `&mut self` and live in one impl block with
`#[methods]`. Helper methods may go in the same block, or in other impl blocks.

| Method | For | Signature |
|---|---|---|
| `read_<p>` | a parameter | `async fn read_value(&mut self) -> Result<T>` |
| `write_<p>` | a writable parameter | `async fn write_target(&mut self, v: T) -> Result<T>`, returning the value actually set |
| `do_<c>` | a command | `async fn do_wait(&mut self, arg: A) -> Result<R>`; without argument or result, `()` |
| `start` | the module | `async fn start(&mut self) -> Result<()>`, run once when the node starts |
| `poll` | the module | `async fn poll(&mut self, due: &[&str]) -> Result<()>`, all parameters due at a poll tick |

**Implicit callbacks** (Frappy's rule): a parameter without `read_<p>` is served from its
stored value and is never polled or read at startup; a writable parameter without
`write_<p>` stores a changed value, publishes it and replies with it. A command needs its
`do_<c>`.

The module handles one request at a time, so methods never overlap. The `start` hook
runs before any other method; requests arriving meanwhile wait for it. Without a `poll`
hook, the due parameters are read one after another with their read methods. Errors are
`SecopError`s; `?` converts any error type that has a `From` conversion into one.

## Parameter values

Every parameter field is a `Param<T>` holding its latest value: the last one read,
written, set, pushed or configured.

- `self.value.get()` returns it.
- `self.status.set(v)` stores and publishes it. Inside a method handling a request, the
  update reaches clients before that request's reply.
- `self.value.handle()` gives a cloneable `ParamHandle<T>` for background tasks (a
  simulation, a hardware callback); values set through it are published at once.

A value a method returns may lie outside the datainfo's limits (a node may report a
measured value out of range), but must fit its type: an array longer than `maxlen` is
published as an `InternalError`.

## Settings and initial values

`typed::module::<X>(name, description, settings)` takes the settings as a JSON object in
code; node config files supply them from a module table. Each property takes its setting
or its `default`; a missing property without a default is an error. A setting that names
a parameter is its configured initial value, checked with all limits:

- a writable parameter is written with its write method when the node starts, after the
  `start` hook and before the startup reads (without a write method it is just stored);
- a readonly parameter without a read method starts with it;
- a readonly parameter with a read method, or a constant one, cannot be configured.

A failing initial write is logged and published as an error update; the node keeps
running. Any other setting is an error. All errors are reported together, each naming
the module and the setting.

## Compile errors you may see

| Error | Cause |
|---|---|
| `` `double` has no key `maxchars` `` | a key of another datainfo type |
| `` the Rust type `String` does not fit a `double` datainfo `` | a wrong `as T` |
| `` `Drivable` needs a command `stop` `` | a missing accessible of the interface class |
| `` `M` has no parameter `vlaue` for `read_vlaue` to read `` | a misspelled method name |
| `` `value` is readonly: `write_value` needs `value` declared `writable` `` | a write method for a readonly parameter |
| `` the command `stop` needs a method `do_stop` `` | a command without method |
| `` `M` needs a `#[must_server::methods]` impl block `` | no methods block (it may be empty) |
| `` expected `i64`, found `f64` `` at a method | a method whose types do not match the declaration |
| conflicting implementations of `Methods` | two methods blocks |
| `` `param!` is only valid in a module declaration `` | `param!` outside a module struct |

The methods block must be in the struct's module or one of its descendants, as it uses
the struct's fields.
