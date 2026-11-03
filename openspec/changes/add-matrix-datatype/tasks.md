# Tasks

## 1. Matrix datainfo in must-types

- [x] 1.1 Add `DataInfo::Matrix(MatrixInfo)` with `names`, `maxlen`, `ElementType { big_endian, kind: I|U|F, bytes }` to `crates/must-types/src/datainfo.rs`. Parse it (mandatory properties, the "Matrix properties" checks, `compression` rejected as unsupported), serialize it, and add a code-built constructor like the other types. Remove `UnsupportedType` if nothing else uses it. Replace the `matrix_not_supported` test with unit tests for every scenario of the `secop-datainfo` delta.
- [x] 1.2 Update the crate docs in `crates/must-types/src/lib.rs` and the header of `tests/spec_examples.rs` so matrix is no longer excluded. Add the specification's matrix datainfo example to `spec_examples.rs`. Verify with `cargo test -p must-types` and `cargo doc -p must-types` without warnings.

## 2. Matrix values in must-types

- [x] 2.1 Add `Value::Matrix { len: Vec<u32>, data: Vec<u8> }`. Decode and encode it per the "Matrix values" requirement, reusing the blob base64 code and using checked multiplication for the byte count. Verify with unit tests for each `secop-values` scenario, including the spec example decoding to 1–6.
- [x] 2.2 Add typed helpers on `MatrixInfo`: element count, `f64` view for integer types and `f4`/`f8`, and building data from `f64`s with range checks per element type. Document that `f2` passes through as bytes only. Verify with unit tests for both endiannesses and for `u8`/`i64` extremes.
- [x] 2.3 Extend the proptest strategies in `tests/roundtrip.rs` to matrix datainfos and values. Verify that `cargo test -p must-types` passes.

## 3. must-server

- [x] 3.1 Make every exhaustive match on `DataInfo`/`Value` in `crates/must-server` handle matrix, including the model's datainfo checks and postfix rules (`_min`/`_max`/`_limits` do not apply to matrix). Add a runtime test with a Rust handler serving a matrix parameter: `describe` shows the datainfo, `read` gives the transport, and an oversized `change` gets `RangeError` without calling the handler. Verify with `cargo test -p must-server`.

## 4. C ABI

- [x] 4.1 In `crates/must-ffi/src/variant.rs`, build the struct node (`len`, `blob`) from a matrix datainfo and from transport JSON with a matrix hint. Keep the matrix datainfo on the node so that `type_json`/`to_datainfo` return it. Verify with unit tests for the "Created from a matrix datainfo" and "Parsed from transport JSON with a hint" scenarios and for a `to_datainfo` round trip.
- [x] 4.2 Add `to_value_direct`/`from_value_direct` to `variant::Node`: matrices move their `len` and bytes, other types fall back to JSON. Change `Answer` to carry an `AnswerValue` (JSON, handle node or matrix). Use the direct path in `Node::to_value`, the getter and command results, `putCommandAnswer`, and requests to setters and the queue. Verify with unit tests that the direct and JSON paths give equal `Value`s and handles for a matrix and for a struct, and that a bad blob size gives the same error both ways.
- [x] 4.3 Export `SECoP_S_updateParameterMatrix` and `SECoP_S_putCommandAnswerMatrix` from `server/api.rs`:
  - name codes as `updateParameter2`;
  - matrix and shape checks before anything is sent or taken out of `handed_out`;
  - one copy of `pData`;
  - with a negative error code, the matrix arguments are not read.

  Verify with unit tests for "Not a matrix parameter", "Size does not fit", "Empty matrix" and "Request without a matrix value". For the latter, check that the request can still be answered afterwards.
- [x] 4.4 Declare both functions in `include/SECoP-exports.h`, marked "must addition, not in SHALL", with Doxygen comments in SHALL's style. Note the LabVIEW array data pointers and that `puLen`/`pData` are not kept. Add calls to `crates/must-ffi-ctests/c/header_check.c` and `header_check.cpp`. Verify the header checks compile as C99 and C++11 and link.
- [x] 4.5 Add C tests to `crates/must-ffi-ctests` (`c/variant.c`, `c/server.c` with drivers in `tests/`):
  - create and inspect a matrix handle;
  - a matrix parameter served by a getter in callback mode;
  - "Matrix parameter in polling mode";
  - "Value that does not fit the datainfo";
  - "Push a matrix", "Answer a read" and "Answer with an error";
  - the same 6-float matrix pushed through `updateParameter`, `updateParameter2` and `updateParameterMatrix`, with the three update lines compared byte for byte.

  Verify they pass on Linux, and that the i686 job runs them.
- [x] 4.6 Document the struct shape, blob access through `SECoP_V_getString`/`modifyString`, the two matrix functions (when to prefer them, no sigma) and the size cost in `COMPATIBILITY.md` (protocol section and additions to SHALL). Verify that the documented calls match the C tests.
- [x] 4.7 Add a matrix case to the benchmarks in `bench/`, or record it in the PR if `bench/` does not fit: push a 512×512 `<f4` matrix through `updateParameter` with a handle and through `updateParameterMatrix`, before and after 4.2. Verify that each path base64-encodes once, using allocation counts or time per update.

## 5. Checks

- [x] 5.1 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `MUST_TUI=0 cargo test --workspace`, and confirm CI passes on Linux x86_64, Windows x86_64 and Windows i686.

## Workflow follow-up

- Archive this change after review.
