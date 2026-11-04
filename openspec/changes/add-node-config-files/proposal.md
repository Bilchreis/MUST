# Proposal

## Why

A MUST node is built in code today: every module is spelled out with `NodeBuilder`, `Module`, `Parameter` and a handler, so adding, renaming or reconfiguring a module means changing and rebuilding a program. Frappy separates *what a module type is* (a class) from *which modules a node has* (a config file), so one driver serves many setups (issue #59). A common setup is many modules of one type, e.g. a gas rack with five mass flow controllers that differ only in name, bus address, gas and range. The config file has to declare these without repeating each module in full. It also has to do this without becoming a program: configs that run code (Frappy's Python cfg files, an embedded scripting language) are left out of this change.

## What Changes

- **Module types.** A new crate `must-config` defines a module type: a named factory (`ModuleType` trait, also implemented for closures) that turns one module's config settings into a `must_server::Module` with parameters, commands and handler. Module types are registered in a `Registry` under names such as `must::sim::FlowController`. The registry is compiled in: a driver collection is a crate that registers its types.
- **Node config files (TOML).** A `[node]` table holds the node properties (`equipment_id`, `description`, `interface = "tcp://2055"`, `firmware`, `implementor`, `timeout`, custom `_` properties, `discovery`, `log_files`). Each `[modules.<name>]` table declares one module: its `type`, `description`, the module properties `group` and `visibility`, custom `_` properties, initial values of cached parameters such as `pollinterval`, and the settings its type reads. Modules are published in file order.
- **Module templates for N modules of one type.** A module whose table key contains a placeholder is a template that expands into several modules of one type, with no scripting:
  - `count = 5` makes five instances; `{i}` is the instance number (from `first`, default 1).
  - `instances = [{ gas = "N2", address = 3 }, …]` makes one instance per entry. Each entry's keys override the template's settings and can be used as placeholders.
  - Placeholders (`{i}`, `{gas}`) are replaced in the module name and in every string setting. A string that is exactly one placeholder takes the variable's TOML type, so `address = "{i}"` gives an integer.
  - Substitution is the whole language: no expressions, conditions or loops.
- **Validation with useful errors.** The config is checked completely before the node is built. Every error is reported at once, with file, line and column and the module it concerns (and the template and instance it came from). This covers unknown or misspelled types and settings, wrong value types, values outside a parameter's datainfo, unknown placeholders, duplicate or invalid expanded names, and the existing `NodeBuilder` completion errors.
- **Runner.** `must serve <node.toml>` builds and serves the node until Ctrl-C. `must check <node.toml>` only validates the config and prints errors and warnings. `must types` lists the registered module types. `must-config` exposes the same entry point (`must_config::cli::main(registry)`), so a driver collection makes its own binary with its own types in a few lines.
- **Built-in simulation types** `must::sim::Temperature` and `must::sim::FlowController`, so configs can be tried and tested without hardware, plus example configs: one cryostat, and a gas rack with five flow controllers declared through a template.
- **`must-server`:** a `Module` can be asked for a parameter's datainfo, and can have the initial value of an already declared cached parameter set by name. The config loader needs this to apply `pollinterval = 2.0` to a module built by a type.

## Capabilities

### New Capabilities
- `module-types`: the module type contract, the registry, and the built-in simulation types.
- `node-config-files`: the TOML node config format, module templates and placeholders, validation and error reporting, and building a `Node` from a config.
- `node-config-runner`: the `serve`, `check` and `types` commands in `must` and for driver-collection binaries.

### Modified Capabilities
- `sec-node-model`: looking up a declared parameter's datainfo, and setting the initial value of a declared cached parameter by name.

## Impact

- New crate `crates/must-config` (depends on `must-server`, `must-types`, `tokio`, `tracing-subscriber`, and new workspace dependencies `toml` and `serde`'s `derive` feature). The new dependency needs a `cargo deny` license check.
- `crates/must-server/src/model.rs`: `Module::parameter_datainfo` and `Module::set_initial_value`.
- `crates/must-tui/src/bin/must.rs`: the `serve` and `check` subcommands and the usage text. `must` then also links `must-server`, so `cargo xtask dist` and the install scripts ship a bigger `must` but no new files.
- Docs: a new `docs/node-config.md` (format, templates, runner, writing a module type), `SUMMARY.md`, the README, and `docs/roadmap.md`.
- Pure Rust with no platform-specific API. It builds and is tested on Linux x86_64, Windows x86_64 and Windows i686 in the existing CI.
- **Out of scope** (left for follow-ups, see design):
  - configs that run code
  - module types that *expand* at startup through hardware discovery (e.g. scanning a Modbus bus)
  - shared IO modules (`io = "bus"`)
  - a derive macro for module types
  - dynamically loaded module types
  - writing initial values of non-cached writable parameters to the hardware at startup
  - reading Frappy cfg files
  - config files for C/LabVIEW nodes (the SHALL C ABI stays code-based)
