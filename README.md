# MUST

MUST is a Rust rewrite of [SHALL](https://github.com/SampleEnvironment/SHALL), the
SECoP hardware abstraction layer. It lets existing programs, drivers and LabVIEW code
offer their hardware as a SEC node speaking [SECoP](https://github.com/SampleEnvironment/SECoP)
2.0, the Sample Environment Communication Protocol. It is a drop-in replacement for
SHALL's C interface, and a library for writing SEC nodes in Rust.

The name is written MUST; `must` in lowercase is the command, and crate names start with
`must-`.

Differences to SHALL:

- written in Rust, without Qt (no Qt build step, library or runtime dependency)
- speaks SECoP 2.0 (draft) instead of SECoP V2019-09-16, including the `matrix` data
  type, `check` (with `SECoP_S_setCheckFunction` and `SECoP_S_ACTION_CHECK`, MUST
  additions) and remote logging (`logging`, with `SECoP_S_logMessage`, a MUST addition)
- a terminal UI (`must-tui`) replaces SHALL's Qt status window

**Status:** early development. All planned features are implemented (see the
[roadmap](docs/roadmap.md)); a manual LabVIEW smoke test and a manual check of the status
TUI on Windows are pending. SHALL's client library (`SECoP_C_*`) is out of scope.

## Getting started

### Replacing SHALL (C and LabVIEW)

1. Download the libraries for your platform from the
   [Releases](https://github.com/Bilchreis/MUST/releases) page.
2. Put `SHALLserver` and `SHALLvariant` in place of SHALL's files (both in one folder).
3. Remove SHALL's Qt libraries.

C programs and LabVIEW VIs that use SHALL's C functions run unchanged.
[COMPATIBILITY.md](COMPATIBILITY.md) describes the differences to SHALL and the functions
MUST adds (`SECoP_S_filterStoredCommand[2]`, `SECoP_S_registerUserError`,
`SECoP_S_setCheckFunction`, …). To build the libraries yourself, see
[Building from source](#building-from-source).

### Writing a SEC node in Rust

Add `must-server` and `must-types` from this repository (they are not on crates.io), and
`async-trait` for the handler trait:

```toml
[dependencies]
must-server = { git = "https://github.com/Bilchreis/MUST", tag = "v0.2.1" }
must-types = { git = "https://github.com/Bilchreis/MUST", tag = "v0.2.1" }
async-trait = "0.1"
```

Declare the node and connect each module to the hardware:

```rust
use async_trait::async_trait;
use must_server::{Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, Parameter, Reading};
use must_types::{DataInfo, Value};

struct Thermometer;

#[async_trait]
impl ModuleHandler for Thermometer {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        let value = match parameter {
            "value" => Value::Double(4.2), // ask the hardware here
            _ => Value::Tuple(vec![Value::Enum(100), Value::String("ok".into())]),
        };
        Ok(Reading::new(value))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let kelvin = DataInfo::double().min(0.0).unit("K").build()?;
    let status = DataInfo::tuple(&[
        DataInfo::enumeration([("IDLE", 100), ("ERROR", 400)]).build()?,
        DataInfo::string().build()?,
    ])
    .build()?;

    let node = NodeBuilder::new("HZB-example", "an example node")
        .module(
            Module::new("t1", "sample thermometer")
                .interface_classes(["Readable"])
                .parameter(Parameter::new("value", kelvin, "temperature"))?
                .parameter(Parameter::new("status", status, "status"))?
                .handler(Thermometer),
        )?
        .build()?;

    let running = node.start()?;
    println!("SEC node listening on {}", running.local_addr());
    // push new values whenever the hardware reports them:
    running.update("t1", "value", Value::Double(4.25))?;
    std::thread::park();
    Ok(())
}
```

A complete simulated temperature controller is in the repository (port 2055):

```bash
cargo run --example simple_node
# in another terminal: nc localhost 2055, then `*IDN?`, `describe`, `activate`
```

Any SECoP client works with it, for example Frappy's `frappy-gui` or `frappy-scan`. The
[API docs of `must-server`](https://bilchreis.github.io/MUST/api/must_server/index.html)
describe the library.

### Installing the tools

`must monitor` opens `must-tui`, the status TUI showing all MUST nodes running for your
user. Install `must` and `must-tui` from a checkout (needs [Rust](https://rustup.rs)):

```bash
./install.sh      # Linux; on Windows: .\install.ps1
must monitor
must update       # later: installs the newest release in this checkout
```

Both also ship next to the libraries in the release archives. See
[the status TUI guide](docs/status-tui.md) for views, keys and `must update`.

## Features

- **Monitoring.** Every process running MUST nodes offers a local, read-only monitoring
  endpoint with the status of its nodes, their clients, message traffic, parameter
  values and log records. `MUST_MONITOR=0` turns it off.
  [Guide](docs/monitoring-protocol.md)
- **Status TUI.** `must-tui` shows all MUST nodes of the user in one terminal window.
  Programs that set SHALL's `bGUI` flag or call `SECoP_S_showStatusWindow(1)` open it in
  a window of its own. [Guide](docs/status-tui.md)
- **Polling.** Modules are polled in two tiers inferred from the interface class
  (`value` and `status` every 5 s, the rest every 60 s); clients can change the rate
  through a `pollinterval` parameter. [Guide](docs/polling.md)
- **Log files.** Every node writes a rolling daily log file of its traffic and log
  records, kept for 2 days. [Guide](docs/log-files.md)
- **Discovery.** Every node answers SECoP's UDP discovery on port 10767 and announces
  itself at startup. [Guide](docs/discovery.md)

## Platforms

| Target | Library files |
|---|---|
| Linux x86_64 | `lib*.so` |
| Windows x86_64 | `*.dll` |
| Windows i686 (32-bit, e.g. 32-bit LabVIEW) | `*.dll` |

CI builds and tests on all three.

## Crates

| Crate | Purpose |
|---|---|
| `crates/must-types` | SECoP 2.0 datainfo, values, identifiers and parameter postfixes |
| `crates/must-server` | SEC node library: declare a node, connect hardware handlers, serve it over TCP |
| `crates/must-monitor` | Monitoring protocol, per-user registry of endpoints, local transport and client library |
| `crates/must-launch` | Opens `must-tui` in a window of its own, for SHALL's status window flags |
| `crates/must-tui` | `must-tui`, the terminal UI showing all MUST nodes of the user, and the `must` command |
| `crates/must-ffi` | SHALL's C interface (`SECoP_S_*`, `SECoP_V_*`), built as `SHALLserver64`/`32` |
| `crates/must-ffi-variant` | `SHALLvariant64`/`32`, forwarding to `SHALLserver` |
| `crates/must-ffi-ctests` | C conformance tests of the C interface |

## Building from source

The toolchain version is pinned in `rust-toolchain.toml`; rustup installs it on first use.

```bash
cargo test --workspace
```

The C tests use SHALL's `bGUI` flag and open the status TUI (on Linux, in a terminal
window when there is a graphical session). Run them with `MUST_TUI=0` to keep it closed,
as CI does.

Build the SHALL-named libraries, their headers, `must` and `must-tui` into
`dist/<target>/`:

```bash
cargo xtask dist                                 # this machine's target
cargo xtask dist --target i686-pc-windows-msvc   # 32-bit Windows, e.g. for 32-bit LabVIEW
cargo xtask check                                # exports, dependencies, cross-library calls
```

How releases are made is described in [docs/releasing.md](docs/releasing.md).

## Documentation

The guide in [docs/](docs/) and the API docs of the crates are published at
<https://bilchreis.github.io/MUST/>. To build them locally:

```sh
cargo doc --workspace --no-deps --open
mdbook build docs  # writes target/book; needs mdbook (cargo install mdbook)
```

## License

MUST is licensed under the GNU Lesser General Public License v3.0 or later
(LGPL-3.0-or-later), the same license as SHALL. See [COPYING.LESSER](COPYING.LESSER) and
[COPYING](COPYING).
