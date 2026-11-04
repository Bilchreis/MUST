# Design

## Context

- A node is declared in code (`must-server/src/model.rs`). `NodeBuilder::new(equipment_id, description)` takes `.bind`, `.firmware`, `.discovery(Discovery)`, `.log_files(LogFiles)` and `.module(Module)`. `Module::new(name, description)` takes `.interface_classes`, `.parameter`, `.command`, `.handler` and the poll settings. `NodeBuilder::build` checks everything and returns a `BuildError` with a list of `ModelError`s, plus warnings on the `Node`.
- `add-module-macro` (implemented first) provides `#[must_server::module]` structs implementing `typed::TypedModule`, constructed from a `typed::Settings` source (`take`, `remaining`, `error`) with property checks, configured initial values and initial writes at start. It also provides `typed::module::<X>(name, description, settings)`, which returns a `Module`.
- `DataInfo::decode(&json, Mode::Request)` (`must-types`) checks a JSON value against a datainfo with all limits. `validate_identifier` and the case-insensitive identifier set are in `must-types/src/identifier.rs`.
- The `must` binary lives in `must-tui` (`src/bin/must.rs`) and has `monitor` and `update`. `examples/simple_node.rs` shows a simulated cryostat and the logging setup (fmt layer plus `must_server::monitor::log_layer()`).
- `toml_edit` 0.25 (with `toml_parser` and `toml_datetime`) is already in `Cargo.lock` as a build dependency, so its licenses have passed `cargo deny`.

## Goals / Non-Goals

**Goals:**
- Declaring N modules of one type in a few lines, with per-instance differences, in a format that stays static data.
- Error messages good enough that a user who has never seen the module type's code can fix the config: location, module, template and instance, expected type.
- A module type is a macro-declared struct, with no config-specific code in the driver. Hand-written factories on the builder API remain possible for modules whose shape depends on the config.

**Non-Goals:**
- Changing the runtime, the protocol, or the C ABI. Everything the runtime needs (start hook, initial writes) comes from `add-module-macro`.
- Listing a type's settings (`must types --verbose`). Macro-declared types have a declaration it could be generated from. It is left for later and needs no config changes.
- Hot reload. The config is read once, before the node starts, because a SECoP description is fixed while the node is up.

## Decisions

### Declaring N modules of one type without scripting: templates with substitution

The problem: five MFCs share type, bus and poll rate, and differ in name, address and gas. Some differences are regular (address = 1..5), others are not (gas, range, serial number). The options were weighed against three things: the config stays data, the module names are visible in the file, and errors can point at a line.

| Option | Verdict |
|---|---|
| Write each module out (plain TOML) | Works today, but is verbose and error-prone for 5–20 modules: a shared change must be made N times. Still allowed. |
| Columnar lists (`address = [1,2,3,4,5]`, `gas = ["N2", …]`) | Rejected. The per-module values are scattered over many arrays, which easily get out of step. The error for a list that is one element short is unclear. |
| **Template key with `count` or `instances`, plus placeholders** | **Chosen.** One table holds the shared settings. Each instance is one inline table holding its differences. The table key (`"mfc{i}"`, `"mfc_{gas}"`) shows the names that will exist. |
| Expressions in placeholders (`{i+2}`, `{i*10}`) | Rejected for now. `first` covers an offset, and anything irregular is written in `instances`. Expressions are where a config language starts turning into a programming language. |
| Cartesian product (a "matrix", as in GitHub Actions) | Rejected. Hardware racks are lists, not grids. A product can be written out in `instances` if it is ever needed. |
| Inheritance between named tables (`extends = "mfc_base"`) | Not now. Templates already share settings within a group. Defaults shared between different groups are rare, and inheritance chains make "where does this value come from" harder to answer in errors. It can be added later without breaking configs. |
| Embedded scripting (Rhai, Lua, Python) | Out of scope by request. Templates do not block it: a scripted config would emit the same instance list. |

How expansion works, in the loader and before any module type runs:

1. A key containing an unescaped `{` is a template. Its table, minus `count`/`instances`/`first`, is the *base*.
2. `count = N` makes N empty instance tables, and `instances = [...]` gives them explicitly. Instance k (0-based) has `i = first + k`.
3. Instance settings are the base, with the instance table's keys replacing keys of the same name. The replacement is shallow: an instance's `[x]` table replaces the base's `x` whole, so it is always clear where a value came from.
4. Variables are `i` plus every key of the instance table. Substitution runs over the module name and every string in the merged settings except those that came from the instance table, so values are never substituted twice. A string that is exactly `{var}` becomes the variable's value with its type. Anywhere else the variable's text is inserted. `{{`/`}}` are escapes.
5. A variable used by a placeholder is marked "template variable". It is exempt from the unknown-setting check unless the type reads it anyway or it names a parameter.

Every setting remembers its *origin*: the span of its value in the file (in the instance table or the base) and, for expanded modules, the template key and instance number. All later errors use the origin, so an error in `mfc3` points at the right line even if the value came from the template.

The format leaves room for the deferred **expanding types** (hardware discovery). A later expansion hook on module types (e.g. a `#[module(..., expand)]` hook) would return instance tables at load time (e.g. one per MFC found on the bus), and they would go through steps 3–5 and the same checks. Configs written now stay valid. Discovery code lives in the driver, not in the config, so the config still does not run code.

### Parsing: `toml_edit` with spans

The `toml_edit` immutable document API (`ImDocument`/`Document::parse`) keeps the key order, which is needed for module order, and gives byte spans for keys and values. It is already in the dependency tree. The loader converts it into its own tree (`Setting { value: toml::Value-like enum, span, origin }`) and maps spans to line and column once. A plain `toml::from_str` into serde structs was rejected because it loses spans and order (`preserve_order` restores order but not spans for nested values). If `toml_edit`'s span API turns out to be insufficient, the fallback is `toml` 0.9's `de::DeTable` with `Spanned` values.

### Module types (`must-config`)

```rust
impl Registry {
    pub fn new() -> Self;                                     // empty
    pub fn default() -> Self;                                 // the must::sim types
    pub fn register<T: TypedModule>(&mut self, name: &str) -> Result<(), RegistryError>;
    pub fn register_fn<F>(&mut self, name: &str, factory: F) -> Result<(), RegistryError>
        where F: Fn(&mut ModuleConfig<'_>) -> Result<Module, ConfigError> + Send + Sync + 'static;
    pub fn names(&self) -> Vec<&str>;                         // sorted
}
```

- **One config view for both kinds.** `ModuleConfig` is the loader's view of one module instance's settings. It implements `typed::Settings`, mapping `take` to its consumed-key tracking and `error` to a `ConfigError` at the key's span, or at the module table's span for a missing key.
  - A macro-declared type is built with `typed::module::<T>(name, description, cfg)`. Property checks, the rules for configured initial values, unknown-setting errors and the initial writes at start all come from `add-module-macro`. The loader only turns its `SettingsErrors` into located `ConfigError`s.
  - A factory reads settings through `get`/`get_or`/`get_opt` (serde `DeserializeOwned`, an integer accepted for a float) and returns a `Module`. Keys it doesn't read are unknown settings.
- **Loader keys.** Before either kind runs, the loader takes out `type`, `description` (a macro type's declared description is the default), `group`, `visibility`, `_*` and the template keys, and applies them on the returned `Module`. Template variables (see templates) are removed from the view, so neither kind sees them.
- **Panics.** Construction runs inside `catch_unwind`. A panicking type becomes a config error for that module instead of aborting the whole check.
- **Driver convention.** A driver collection exposes `pub fn register(r: &mut Registry) -> Result<(), RegistryError>`.

*Alternative considered:* factories only, with a derive on top later. This was the first version of this design. It was dropped once `add-module-macro` gave types a declaration: with that, the config needs no type-specific code at all, and configured initial values and initial writes work the same for every macro type.

### Errors

`ConfigError { file, line, column, module: Option<String>, origin: Option<(template, instance)>, message }`, collected in a `ConfigErrors(Vec<ConfigError>)` that is `Display`-ed one per line in the `path:line:col: module mfc3 (template "mfc{i}", instance 3): …` form that editors can jump to. The loader goes on past errors where the result is still meaningful:
- A broken module is skipped and the others are still built.
- `NodeBuilder::build` runs only if there were no config errors. Its `ModelError`s are mapped back to the module's table span through the module name.

Warnings use the same form.

### Node table

- `interface` mirrors Frappy's `tcp://<port>`, so Frappy users find their way around. `tcp://<host>:<port>` is accepted too.
- A relative `log_files.dir` resolves against the config file's directory, not the current directory, so `must serve` works from anywhere.
- `firmware` defaults to `must-<version>` with the `must-config` crate version.

### Runner placement

`must_config::cli::run(registry, args) -> ExitCode` implements `serve`/`check`/`types`, and `cli::main(registry)` reads `std::env::args`. The `must` binary (in `must-tui`) dispatches these three subcommands to it with `Registry::default()`. `serve` starts a multi-thread tokio runtime, spawns the node on it, waits for `tokio::signal::ctrl_c` (which works on Linux and Windows) and calls `stop`.

*Alternative considered:* a separate `must-node` binary. Rejected because `must` is already the one command users install and know, and it keeps `serve` next to `monitor`. The cost is a larger `must` executable, which is acceptable.

### Simulation types

Both are `#[must_server::module(Drivable)]` structs in `must-config::sim`, and the first in-tree users of the macro.
- **`must::sim::Temperature`** is the example cryostat (properties `temperature`, the initial temperature, and `ramp`).
- **`must::sim::FlowController`** is a first-order flow towards `target`, with properties `range` (required), `unit` (default `ln/min`) and `settling_time` (default 2 s). Its datainfos name these properties (`target: param!("setpoint", double(min = 0.0, max = range, unit = unit), writable)`), so each instance gets its own limit and unit.
- **State:** each module keeps its state in plain fields, advanced by the elapsed time in every read or write method, so there are no background tasks.
- **Polling:** both declare a writable `pollinterval` without methods (an implicit callback), so `pollinterval = …` in a config works, and `target = …` exercises the initial write.

## Risks / Trade-offs

- [Placeholder syntax clashes with values that contain braces, e.g. a C format string] → `{{`/`}}` escapes, and placeholders are only active in template tables. A plain `[modules.x]` never substitutes.
- [Typed "exactly one placeholder" substitution is implicit] → It is documented with the bus-address example, and the alternative (a string `"3"` where an integer is needed) would be a type error anyway.
- [Factories read settings imperatively, with no up-front list of their settings] → Unknown-setting errors still catch typos after construction. Factories are the escape hatch, and macro types are the common case.
- [Module type code runs during `check`] → Construction must not touch hardware. Macro types connect in their `start` hook, which `check` never runs, and factories connect on first use. This is documented as the module type contract.
- [`must` grows by the server library] → Accepted (see Runner placement).
- [The TOML 1.1 features `toml_edit` 0.25 accepts (e.g. newlines in inline tables)] → Fine for users. The docs' examples stick to TOML 1.0.

## Migration Plan

Additive. Existing code-built nodes and the C ABI are unaffected. `examples/simple_node.rs` stays as the "code-first" example, and the docs point to `must serve` with `examples/cryostat.toml` as the config-first alternative. No rollback is needed beyond reverting the crate.

## Open Questions

- Whether to accept a `bind = "host:port"` alias besides `interface` for users who never used Frappy. This can be added later without breaking configs.
- Exact wording and order of errors in `must check` output (e.g. sorted by line vs. by module). This is settled during implementation with the snapshot tests.
