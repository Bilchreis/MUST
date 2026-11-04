# Design

## Context

- A node is declared in code (`must-server/src/model.rs`). `NodeBuilder::new(equipment_id, description)` takes `.bind`, `.firmware`, `.discovery(Discovery)`, `.log_files(LogFiles)` and `.module(Module)`. `Module::new(name, description)` takes `.interface_classes`, `.parameter`, `.command`, `.handler` and the poll settings. `NodeBuilder::build` checks everything and returns a `BuildError` with a list of `ModelError`s, plus warnings on the `Node`.
- A `Module` can only be built up. It has no accessor for a declared parameter's datainfo, and no way to change a parameter after `.parameter(...)`.
- `DataInfo::decode(&json, Mode::Request)` (`must-types`) checks a JSON value against a datainfo with all limits. `validate_identifier` and the case-insensitive identifier set are in `must-types/src/identifier.rs`.
- The `must` binary lives in `must-tui` (`src/bin/must.rs`) and has `monitor` and `update`. `examples/simple_node.rs` shows a simulated cryostat and the logging setup (fmt layer plus `must_server::monitor::log_layer()`).
- `toml_edit` 0.25 (with `toml_parser` and `toml_datetime`) is already in `Cargo.lock` as a build dependency, so its licenses have passed `cargo deny`.

## Goals / Non-Goals

**Goals:**
- Declaring N modules of one type in a few lines, with per-instance differences, in a format that stays static data.
- Error messages good enough that a user who has never seen the module type's code can fix the config: location, module, template and instance, expected type.
- Writing a module type as plain Rust on top of the existing `Module`/`ModuleHandler` API, with no new runtime machinery.

**Non-Goals:**
- Changing the runtime, the protocol, or the C ABI.
- A settings schema that a type declares up front (for documentation or `must types --verbose`). Types read their settings imperatively. A schema can be added later without changing configs.
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

The format leaves room for the deferred **expanding types** (hardware discovery). A later `ModuleType::expand` would return instance tables at load time (e.g. one per MFC found on the bus), and they would go through steps 3–5 and the same checks. Configs written now stay valid. Discovery code lives in the driver, not in the config, so the config still does not run code.

### Parsing: `toml_edit` with spans

The `toml_edit` immutable document API (`ImDocument`/`Document::parse`) keeps the key order, which is needed for module order, and gives byte spans for keys and values. It is already in the dependency tree. The loader converts it into its own tree (`Setting { value: toml::Value-like enum, span, origin }`) and maps spans to line and column once. A plain `toml::from_str` into serde structs was rejected because it loses spans and order (`preserve_order` restores order but not spans for nested values). If `toml_edit`'s span API turns out to be insufficient, the fallback is `toml` 0.9's `de::DeTable` with `Spanned` values.

### Module type API (`must-config`)

```rust
pub trait ModuleType: Send + Sync + 'static {
    fn build(&self, cfg: &mut ModuleConfig<'_>) -> Result<Module, ConfigError>;
    fn default_description(&self) -> Option<&str> { None }
}
impl<F> ModuleType for F where F: Fn(&mut ModuleConfig<'_>) -> Result<Module, ConfigError> + Send + Sync + 'static { … }

impl ModuleConfig<'_> {
    pub fn name(&self) -> &str;
    pub fn new_module(&self) -> Module;                 // Module::new(name, description)
    pub fn get<T: DeserializeOwned>(&mut self, key: &str) -> Result<T, ConfigError>;
    pub fn get_or<T: DeserializeOwned>(&mut self, key: &str, default: T) -> Result<T, ConfigError>;
    pub fn get_opt<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>, ConfigError>;
    pub fn error(&self, key: &str, message: impl Display) -> ConfigError; // located at the key
}
```

- Settings are read through serde (`T: DeserializeOwned`) from the loader's value tree, so a type can read a whole `#[derive(Deserialize)]` struct from a sub-table. Reading a key marks it consumed. An integer is accepted where a float is expected (spec: *Integer for a float*).
- The loader passes the type only the type's own settings. `type`, `description`, `group`, `visibility`, `_*` and the template keys are taken out first and applied by the loader on the returned `Module` (`.group`, `.visibility`, `.property`). The type cannot misread them, and they work the same for every type.
- After `build`, each remaining key is handled in order: template variables are dropped; keys naming a parameter go through `Module::parameter_datainfo` → `DataInfo::decode(Mode::Request)` → `Module::set_initial_value`; anything else is an unknown setting. A type that reads `pollinterval` itself has consumed it, so the type wins.
- `build` runs inside `catch_unwind`. A panicking type becomes a config error for that module instead of aborting the whole check.
- The returned `Module`'s name must equal `cfg.name()`. This needs a `Module::name()` accessor, and a mismatch is reported as a bug in the type.
- `Registry::default()` holds the `must::sim` types and `Registry::new()` is empty. A driver collection exposes `pub fn register(r: &mut Registry) -> Result<(), RegistryError>` by convention.

*Alternative considered:* a trait with associated `Settings: Deserialize` and a `build(name, settings)` call. It is cleaner for simple types, but serde's errors for unknown fields and the field's location would have to be wired through `deny_unknown_fields` and spans per type, and it cannot express "unread key that is a parameter". The imperative `ModuleConfig` keeps all of that in the loader. A derive macro on top of it is a follow-up.

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

`must::sim::Temperature` is the example cryostat moved into `must-config` (settings: `temperature` initial value, `ramp`). `must::sim::FlowController` is a first-order flow towards `target` (settings: `range` required, `unit`, `settling_time` default 2 s). Each module gets its own state behind a `Mutex`, advanced by the elapsed time whenever the handler reads or changes it, so there are no background tasks and no hook is needed to run code after the node starts. Polling keeps clients up to date. Both use `.poll_interval` and a cached `pollinterval` parameter, so `pollinterval = …` in a config works.

## Risks / Trade-offs

- [Placeholder syntax clashes with values that contain braces, e.g. a C format string] → `{{`/`}}` escapes, and placeholders are only active in template tables. A plain `[modules.x]` never substitutes.
- [Typed "exactly one placeholder" substitution is implicit] → It is documented with the bus-address example, and the alternative (a string `"3"` where an integer is needed) would be a type error anyway.
- [Imperative settings reading gives no up-front list of a type's settings] → Unknown-setting errors still catch typos after `build`. A declared schema is a non-goal now, and can be added as an optional `ModuleType::settings()` later.
- [Module type code runs during `check`] → Types must not touch hardware in `build`. They connect on first use or in their own task. This is documented as the module type contract, and the sim types follow it.
- [`must` grows by the server library] → Accepted (see Runner placement).
- [The TOML 1.1 features `toml_edit` 0.25 accepts (e.g. newlines in inline tables)] → Fine for users. The docs' examples stick to TOML 1.0.

## Migration Plan

Additive. Existing code-built nodes and the C ABI are unaffected. `examples/simple_node.rs` stays as the "code-first" example, and the docs point to `must serve` with `examples/cryostat.toml` as the config-first alternative. No rollback is needed beyond reverting the crate.

## Open Questions

- Whether to accept a `bind = "host:port"` alias besides `interface` for users who never used Frappy. This can be added later without breaking configs.
- Exact wording and order of errors in `must check` output (e.g. sorted by line vs. by module). This is settled during implementation with the snapshot tests.
