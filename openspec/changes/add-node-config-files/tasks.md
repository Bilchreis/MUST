# Tasks

## 1. Setup

- [x] 1.1 Check that `add-module-macro` is implemented (`typed::Settings`, `TypedModule`, initial writes). Create `crates/must-config` (lib, `LGPL-3.0-or-later` SPDX headers, workspace lints) and add `toml_edit` (feature `parse`, no `display`) and serde `derive` to `[workspace.dependencies]`. Verify with `cargo build -p must-config` and `cargo deny check` passing.

## 2. Module types and registry

- [x] 2.1 Implement `ModuleConfig` (consumed-key tracking; `get`/`get_or`/`get_opt` through serde; `error`), its `typed::Settings` implementation (errors at the key's span, or at the module table for a missing key), and `ConfigError`/`ConfigErrors` with locations, including the mapping of `typed::SettingsErrors`. Verify with unit tests for *Missing required setting*, *Wrong value type*, *Integer for a float* and *Type rejects a setting* (factory), and *Property from the config* and *Property value too long* (a test macro type).
- [x] 2.2 Implement `Registry` (`new`, `default`, `register::<T: TypedModule>`, `register_fn`, `ident(::ident)*` name checks, duplicate rejection, `names()` sorted). Verify with tests for *Duplicate type name* and *Invalid type name*.
- [x] 2.3 Write `must::sim::Temperature` and `must::sim::FlowController` with `#[must_server::module(Drivable)]` (properties `range`, `unit` and `settling_time` used in the datainfos; elapsed-time simulation; writable `pollinterval` without methods; a `#[must_server::methods]` block each), and register both in `Registry::default()`. Verify with runtime tests that serve a config: *Flow controller range* and *Simulated flow follows the target* (paused tokio time), and *Two instances of one type*.

## 3. Config loader

- [x] 3.1 Parse the file with `toml_edit` into the loader's value tree with spans and line/column mapping, and check the top-level structure (`[node]`, `[modules]`, no other keys, at least one module). Verify with tests for *Minimal config*, *Unknown top-level table* and *Not TOML*. If `toml_edit` spans are unusable, switch to `toml` 0.9 `DeTable` as noted in the design.
- [x] 3.2 Load the `[node]` table: properties, custom `_` properties, `interface` (`tcp://port`, `tcp://host:port`), `discovery`, `log_files` (relative `dir` resolved against the config's directory), and the `firmware` default. Verify with tests for *Custom node property*, *Missing equipment_id*, *Port only*, *Unsupported scheme*, *Discovery off* and *Log file retention*.
- [x] 3.3 Expand templates: detect placeholder keys; check `count`/`instances`/`first`; merge base and instance settings shallowly with origins; substitute (typed when the string is a single placeholder, text otherwise, `{{`/`}}` escapes, never in instance values); mark template variables. Verify with unit tests for every scenario under *Module templates*, *Template instances*, *Placeholder substitution* and *Typed placeholders*.
- [x] 3.4 Check names with `validate_identifier` and a case-insensitive set across direct and expanded modules, with errors naming both declarations. Verify with tests for *Template collides with a module* and *Invalid expanded name*.
- [x] 3.5 Build each module: look up the type (an unknown type lists the registered names); take out the loader keys (`type`, `description` with the type's default, `group`, `visibility`, `_*`) and the template variables; build it under `catch_unwind`, with `typed::module::<T>` for macro types or the factory for `register_fn` types; report unread keys of factories as unknown settings; apply the loader keys. Then run `NodeBuilder::build` in file order and map `ModelError`s and warnings back to module spans. Verify with tests for *Custom module property*, *Unknown type*, *File order kept*, *Poll interval from the config*, *Initial value out of range*, *Target written at start*, *Value of a hardware parameter*, *Misspelled setting*, *Several errors at once*, *Error in an expanded module* and *Warnings do not fail loading*. Use snapshot tests (`insta`) for the error text.
- [x] 3.6 Add `crates/must-config/examples/cryostat.toml` and `gas-rack.toml` (five flow controllers from one template, with per-instance `gas`, `range` and `_serial_number`), and a test that loads both with `Registry::default()`.

## 4. Runner

- [x] 4.1 Implement `must_config::cli::{run, main}` with `serve` (tokio runtime, logging with `RUST_LOG` or `info` plus `monitor::log_layer()`, prints the address, stops on Ctrl-C), `check` (errors and warnings; on success one `module type` line per module) and `types`, with the exit codes from the spec. Verify with integration tests that run the commands on the example configs and on broken configs: *Valid config*, *Errors are reported*, *Built-in types*, *Invalid config* (no port opened), *Port in use*, and *Custom binary* (a test binary that registers `acme::Valve`).
- [x] 4.2 Add `serve`, `check` and `types` to `must` (`crates/must-tui/src/bin/must.rs`) with the usage text, delegating to `cli::run` with `Registry::default()`. Verify with `cargo run --bin must -- check crates/must-config/examples/gas-rack.toml`, and by hand: `must serve` on the gas rack, `nc` to the printed port, `describe` lists `mfc1` to `mfc5`, and `must monitor` shows the node and its log (*Serving the gas rack example*, *Node in the status TUI*).

## 5. Documentation

- [x] 5.1 Write `docs/node-config.md`: the format, templates with the five-MFC example, placeholders and escapes, initial values, error output, `must serve`/`check`/`types`, writing a module type (the macro, with a link to `docs/writing-modules.md`, and factories) and a driver-collection binary, and the out-of-scope items (expanding types, IO modules, configs that run code). Link it in `docs/SUMMARY.md`, the README and `docs/roadmap.md`, and add a reference to issue #59. Verify that `mdbook build docs` succeeds and that the documented commands run as written.

## 6. Integration

- [x] 6.1 In CI (`.github/workflows/ci.yml`), run `must check` on both example configs for each target (Linux x86_64, Windows x86_64, Windows i686) next to the existing `simple_node` smoke test. Verify that the CI run is green on all three targets.
- [x] 6.2 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, and cross-check the `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc` builds as CI does.

## Workflow follow-up

- Archive the change after review and merge, and sync the new `module-types`, `node-config-files` and `node-config-runner` specs into `openspec/specs/`.
- Open follow-up issues: expanding module types (discovery), shared IO modules, a derive macro for module types, and writing initial values to hardware at startup.
