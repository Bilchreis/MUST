# Design

## Context

The repository is empty apart from OpenSpec scaffolding. The reference implementation is SHALL (C++/Qt, LGPL-3.0-or-later): its VariantLib (`SECoP-Variant.h/.cpp`, ~10k lines) models a SECoP value and its datainfo as one object (`CSECoPbaseType`) through a deep virtual-inheritance hierarchy, built on nlohmann::json with a custom ordered map. SHALL targets SECoP V2019-09-16 (v1.0); `must` targets the SECoP 2.0 draft (new `*IDN?` format, `visibility`/`meaning` changes), but leaves out the 2.0 additions `matrix` and `check` for now. See proposal.md for motivation and the roadmap.

Later changes add a SEC node server, the SHALL-compatible C ABI (`SECoP_V_*`, `SECoP_S_*`), node monitoring and the status TUI on top of this crate. Everything must build for Linux x86_64, Windows x86_64 and Windows i686, without Qt.

## Goals / Non-Goals

**Goals:**
- One pure-Rust crate, `must-types`, with no networking, async runtime or FFI dependency.
- Invalid datainfos cannot exist as values of the Rust types (validate on construction).
- Errors precise enough to become SECoP `error_*` replies without further mapping.

**Non-Goals:**
- Message framing, data reports/qualifiers, error reports, properties of modules/accessibles (next change: `add-sec-node-server`).
- The C ABI and its self-describing variant object (change `add-shall-c-abi`).
- Unit handling or `fmtstr` rendering of values.
- The `matrix` data type. `type: "matrix"` is recognized only to give an "unsupported type" error instead of "unknown type". `DataInfo` and `Value` are `#[non_exhaustive]` so adding a `Matrix` variant later is not a breaking change for downstream crates.
- The optional `check` message; request mode is defined so it can serve `check` unchanged once that is added.

## Decisions

### Workspace layout
`Cargo.toml` workspace with members under `crates/`. This change adds `crates/must-types`. Planned later: `must-server`, the C ABI cdylibs (producing `SHALLserver64`/`32` and `SHALLvariant64`/`32`), `must-monitor` (local monitoring endpoint and discovery) and the `must-tui` binary. Edition 2024, `resolver = "3"`, shared lints and versions in `[workspace.*]`.
*Alternative:* one crate with features. Rejected: the FFI cdylib and the async server have very different dependency sets, and separate crates keep `must-types` light for embedded/other users.

### Datainfo and value are separate types
`DataInfo` is an enum with one variant per SECoP type, each holding a struct with private fields (e.g. `IntInfo { min, max, unit, extra }`). `Value` is a separate enum (`Double(f64)`, `Int(i64)`, `Scaled(i64)`, `Enum(i64)`, `Bool`, `String`, `Blob(Vec<u8>)`, `Array(Vec<Value>)`, `Tuple(Vec<Value>)`, `Struct(IndexMap<String, Value>)`). Decoding is `DataInfo::decode(&serde_json::Value, Mode) -> Result<Value, ValueError>`; encoding is `DataInfo::encode(&Value) -> Result<serde_json::Value, ValueError>`.
*Alternative:* SHALL's combined value+type object. Rejected for the core: the datainfo is fixed per accessible while values change constantly; keeping them apart avoids cloning type info with every update. The C ABI change will wrap `(Arc<DataInfo>, Value)` in an opaque handle to reproduce `CSECoPbaseType` semantics.

### Validate on construction, hand-written (de)serialization
Parsing goes `serde_json::Value` → validated `DataInfo` with explicit checks rather than `#[derive(Deserialize)]`, because the checks are cross-field (min ≤ max, optional ⊆ members) and errors must name the offending property. Builders for code-constructed datainfos call the same validation functions. `Serialize` is implemented by hand to emit only given properties plus preserved unknown ones (stored in an `extra: serde_json::Map`).

### Ordered maps
`indexmap` for enum and struct members and `serde_json` with `preserve_order`, so descriptions round-trip in their given order (matters for `describe` output and GUIs).

### Two decoding modes
`Mode::Request` (enforce limits, allow absent optional struct members) and `Mode::Report` (shape only, all members required), as in specs/secop-values. Absent optional members are simply missing from the `Struct` map; the server change decides how to fill them from the current value.

### Dynamic limits as a decode option
Request-mode decoding takes an optional `DynLimits { min: Option<Value>, max: Option<Value> }` (exposed as e.g. `DataInfo::decode_request(&json, Option<&DynLimits>)`). The data-types crate does not know which parameters exist; the server looks up the current `_min`/`_max`/`_limits` values and passes them in. Limits are in transport units, so a scaled parent compares transported integers and no float rounding enters the check. The error message names which limit (static or dynamic) was hit.
*Alternative:* folding dynamic limits into a modified copy of the datainfo. Rejected: datainfos are immutable and shared, while the limits change at runtime.

### Postfix rules live next to datainfo
A small `postfix` module offers `split_postfix(name) -> Option<(&str, Postfix)>`, `Postfix::expected_datainfo(&parent) -> Result<DataInfo, PostfixError>` and `Postfix::check_declared(&parent, &declared)`. Matching compares datainfos with a `matches()` method that checks type and SECoP-defined data properties but ignores `extra` (unknown/custom properties), so `PartialEq` stays a strict equality used by tests. Module-level rules (parent exists, `_limits` vs `_min`/`_max` exclusivity) need the module model and go into `add-sec-node-server`.

### Error types
`thiserror`-based `DataInfoError { path, kind }` for description problems and `ValueError { class: ErrorClass, path, message }` for values, with `ErrorClass::{WrongType, RangeError}`. `path` is a list of segments (`Index(usize)`, `Member(String)`, `Argument`, `Result`) rendered like `[2].x`. The wider `ErrorClass` enum of all SECoP error classes moves into this crate in the server change.

### License
`must` uses the same license as SHALL: LGPL-3.0-or-later. Every crate sets `license = "LGPL-3.0-or-later"` (inherited from `[workspace.package]`), the repository ships the LGPL-3.0 text as `COPYING.LESSER` and the GPL-3.0 text it builds on as `COPYING`, and source files carry an `SPDX-License-Identifier: LGPL-3.0-or-later` header as SHALL's do. This also settles the derived-work question for the later SHALL-compatible C ABI. Dependencies must be LGPL-compatible (MIT/Apache-2.0 are), checked with `cargo deny`.

### Platforms and CI
CI runs on a matrix of `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc` (the i686 build runs natively on the 64-bit Windows runner, so tests execute there too). Fmt, clippy and the license check run once on Linux; build and tests run on all three. Starting with this crate, wire limits (`maxlen`, `maxbytes`, `maxchars`, …) are stored as `u64`, not `usize`, so a description that is valid on 64-bit is also valid on 32-bit; comparisons convert actual lengths up to `u64`.
*Alternative:* cross-compiling i686 from Linux with only a build check. Rejected: 32-bit LabVIEW is a primary user, so the tests have to run on that target.

### Dependencies
`serde`, `serde_json` (`preserve_order`), `indexmap`, `base64` (STANDARD engine, padded), `thiserror`. Dev: `proptest` for round-trip properties. All are pure Rust and build on every target without system libraries.

### Testing
Unit tests per data type next to the code; table-driven tests in `tests/` using the examples from the SECoP 2.0 `datainfo` chapter (except `matrix`) as fixtures; proptest generators for valid datainfo/value pairs to check `decode(encode(v)) == v`.

## Risks / Trade-offs

- [Prose and machine-readable schema disagree on the scaled `fmtstr` default: the `datainfo` chapter says `%.<n>f` derived from `scale`, `schema/datatypes.yaml` says `%.6g`] → Follow the prose, which is specific to scaled and gives the derivation; note the conflict in the crate docs and raise it upstream with the SECoP committee.
- [The spec does not say whether an enum, tuple or struct may be empty] → Accept empty ones, as nothing in the spec forbids them; a server-level warning can be added later if needed.
- [Whether `_limits`/`_min`/`_max` datainfos must match the parent exactly is only stated as "must match" (prose) and `datainfo: parent` (schema)] → Require equal type and SECoP-defined properties, ignore custom ones. Stricter than necessary is easier to relax later than the reverse.
- [SECoP 2.0 is still a draft and may change] → Pin the spec commit used (SampleEnvironment/SECoP `a6e90eb`, 2026-07-07) in the crate docs and keep type-specific logic isolated per variant so changes stay local.
- [Integral floats (`3.0`) accepted for int is more lenient than strictly necessary] → Matches what LabVIEW and Python clients commonly send; costs nothing on the encode side, which always emits integers.
- [A user may want to offer a `matrix` parameter] → Rejected with a clear "unsupported" error for now; `matrix` is first on the roadmap's later list and slots in as a new enum variant.

## Open Questions

- Minimum supported Rust version: default to the version current when implementation starts; revisit if a downstream user needs older toolchains.
