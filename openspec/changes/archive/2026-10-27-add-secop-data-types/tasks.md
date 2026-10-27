# Tasks

## 1. Workspace setup

- [x] 1.1 Create the root `Cargo.toml` workspace (edition 2024, resolver 3, shared `[workspace.package]`, `[workspace.dependencies]` and `[workspace.lints]` with clippy pedantic warnings) and an empty `crates/must-types` library; verify `cargo build --workspace` succeeds
- [x] 1.2 Add `rust-toolchain.toml`, `rustfmt.toml` and a `.gitignore` for `target/`; verify `cargo fmt --check` passes
- [x] 1.3 Add the license: `COPYING.LESSER` (LGPL-3.0 text), `COPYING` (GPL-3.0 text), `license = "LGPL-3.0-or-later"` in `[workspace.package]` inherited by every crate, and a `deny.toml` allowing only LGPL-compatible dependency licenses; verify `cargo metadata` reports the license for `must-types` and `cargo deny check licenses` passes
- [x] 1.4 Add a GitHub Actions workflow: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo deny check licenses` on Linux, plus `cargo build` and `cargo test --workspace` on a target matrix of `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`; verify all jobs pass on a push and the i686 job actually runs the tests
- [x] 1.5 Write a top-level `README.md` describing must (Rust rewrite of SHALL without Qt, SECoP 2.0 target, supported platforms, crate layout, roadmap incl. C ABI drop-in and status TUI, LGPL-3.0-or-later license); verify it links to SHALL and the SECoP spec

## 2. Identifiers

- [x] 2.1 Implement identifier validation for module, accessible and property names (syntax, 63-char limit counting postfixes, custom-name detection) with unit tests for every scenario in specs/secop-identifiers; verify `cargo test -p must-types identifier` passes
- [x] 2.2 Implement a name scope type that preserves case and rejects case-insensitive duplicates (used for identifiers and for struct/enum member names), with unit tests for the clash and lookup scenarios; verify tests pass

## 3. Datainfo model and parsing

- [x] 3.1 Define `DataInfo` and per-type structs, `DataInfoError` with property paths, and the shared parsing helpers (type dispatch, mandatory property, property type checks); verify unknown/missing `type`, non-object inputs and the unsupported `matrix` type are rejected by tests
- [x] 3.2 Implement `double`, `scaled`, `int` and `bool` parsing with limit consistency, positive scale and `fmtstr` syntax checks plus default accessors; verify the numeric, fmtstr and scaled-default scenarios pass as tests
- [x] 3.3 Implement `enum`, `string` and `blob` parsing (enum names non-empty and unique case-insensitively without identifier syntax, unique values, bidirectional lookup, empty enums accepted, length bounds stored as `u64`); verify the enum and length scenarios pass as tests, including a `maxbytes` above `u32::MAX` that must parse on i686
- [x] 3.4 Implement `array`, `tuple`, `struct` and `command` parsing with nested datainfos, rejection of nested `command`, case-insensitively unique struct member names, and `optional` ⊆ members; verify the structured-type scenarios pass as tests
- [x] 3.5 Implement serialization (given properties only, preserved unknown properties, member order) and builders sharing the validation code; verify round-trip, custom-property and built-in-code scenarios pass as tests
- [x] 3.6 Add `tests/spec_examples.rs` parsing every non-matrix datainfo example from the SECoP 2.0 `datainfo` chapter; verify all parse and round-trip

## 4. Values

- [x] 4.1 Define `Value`, `Mode`, `ErrorClass` and `ValueError` with path rendering (e.g. `[2].x`); verify path formatting with unit tests
- [x] 4.2 Implement decode/encode for `double`, `int`, `scaled` (incl. physical conversion), `enum` and `bool` in both modes; verify the numeric, scaled, enum and bool scenarios in specs/secop-values pass as tests
- [x] 4.3 Add optional dynamic limits to request-mode decoding of `double`, `scaled` and `int`, with errors naming the limit that was hit; verify the dynamic-limits scenarios in specs/secop-values pass as tests
- [x] 4.4 Implement decode/encode for `string` (code-point length, ASCII check) and `blob` (padded standard base64); verify the string and blob scenarios pass as tests
- [x] 4.5 Implement decode/encode for `array`, `tuple` and `struct` with error paths and optional members per mode; verify the array, tuple, struct and nested-path scenarios pass as tests
- [x] 4.6 Implement command argument/result decoding and refusal to encode non-finite doubles or mismatched shapes; verify the command and NaN scenarios pass as tests
- [x] 4.7 Add proptest generators for valid (datainfo, value) pairs and a property test that `decode(encode(v)) == v` in report mode; verify `cargo test -p must-types` passes

## 5. Parameter postfixes

- [x] 5.1 Implement postfix name splitting for `_min`, `_max`, `_limits` and `_enable` (custom parents allowed, empty parent rejected); verify the name-recognition scenarios in specs/secop-parameter-postfixes pass as tests
- [x] 5.2 Implement `DataInfo::matches()` (type and SECoP-defined properties, custom properties ignored), the scalar-numeric parent check, expected-datainfo derivation and the declared-datainfo check with an error naming the mismatching member; verify the remaining postfix scenarios pass as tests

## 6. Documentation and integration check

- [x] 6.1 Write crate-level rustdoc for `must-types` with a runnable example (parse a datainfo, decode a request value with dynamic limits, handle a `RangeError`), noting the pinned SECoP spec commit and the scaled `fmtstr` default conflict between prose and schema; verify `cargo test --doc -p must-types` and `cargo doc --no-deps` succeed without warnings
- [x] 6.2 Run the full CI command set locally (`fmt --check`, `clippy -D warnings`, `test --workspace`, `deny check licenses`) and verify all pass; verify the CI matrix is green on all three targets

## Workflow follow-up

- Archive this change after review, then propose `add-sec-node-server` as the next change.
