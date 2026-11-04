# Proposal

## Why

SECoP 2.0 defines a `matrix` data type for medium to large homogeneous, multi-dimensional data such as detector images or spectra. must rejects it today (`DataInfoErrorKind::UnsupportedType`), so a node cannot describe such a parameter. Sending the data as an `array` of `double` costs about ten times as many bytes and has no notion of dimensions. Matrix was deferred when must started, and it is the last SECoP 2.0 data type must lacks.

## What Changes

- **must-types parses `matrix` datainfos**: the mandatory `names`, `maxlen` and `elementtype` properties, checked for consistency. `elementtype` is `<` or `>`, then `i`, `u` or `f`, then a byte count of 1, 2, 4 or 8; `f1` is rejected. The optional `compression` property is rejected as unsupported, since SECoP defines no compression yet.
- **must-types decodes and encodes matrix values**: the SECoP transport `{"len": [...], "blob": "<base64>"}`. It checks that `len` has one entry per dimension, within `maxlen`, and that the blob holds exactly as many bytes as the lengths and element size require. The bytes are passed through unchanged. must does not interpret the elements.
- **Rust handlers** get a matrix value: the lengths and raw bytes, plus helpers that read and write elements of the declared element type.
- **C ABI, no new exports or enum values**: a matrix value crosses the ABI as a variant `SECoP_VT_STRUCT` with two members:
  - `len`: `SECoP_VT_ARRAY_INTEGER`
  - `blob`: `SECoP_VT_BLOB`, the raw bytes
  - `SECoP_V_create` with a matrix datainfo and `SECoP_V_fromJSON` with the SECoP transport JSON both produce this shape.
  - `SECoP_S_updateParameter2` and `SECoP_S_putCommandAnswer2` accept the transport JSON directly.
- **Matrix data without JSON inside must**: must-ffi converts a matrix variant to a must-types value and back by moving the byte buffer. Today's path through JSON would base64-encode the data twice and decode it once on every update. With the direct conversion, the only base64 encoding left is the one for the wire.
- **C ABI, two must additions (not in SHALL) for large or frequent matrices**:
  - `SECoP_S_updateParameterMatrix(szParameterName, puLen, iDims, pData, iDataSize, dblTimestamp)` pushes a matrix update.
  - `SECoP_S_putCommandAnswerMatrix(llId, iErrorCode, puLen, iDims, pData, iDataSize, dblTimestamp)` answers a polled request with a matrix value.
  - Both take the host's lengths and raw element bytes as plain pointers, so no variant is built. The host's buffer is read once, straight into the base64 encoder.
  - LabVIEW's Call Library node can pass its arrays as data pointers, so a VI does not have to build a struct variant member by member.
- **Docs**: `COMPATIBILITY.md` lists matrix support, which SHALL lacks. It describes how matrix values map to variants and documents the two matrix functions.

No breaking change. A datainfo that was rejected before is now accepted, and two functions are added. Nothing else changes.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `secop-datainfo`: "Supported data types" is replaced by "Recognized data types", which includes `matrix` (OpenSpec cannot drop the scenario "Matrix not supported yet" from a modified requirement). "Mandatory data properties" covers `names`, `maxlen` and `elementtype`. New requirement: "Matrix properties".
- `secop-values`: new requirement "Matrix values".
- `shall-variant-abi`: new requirement "Matrix values as structs".
- `shall-server-abi`: new requirements "Pushed matrix updates" and "Matrix answers to polled requests".

## Impact

- `crates/must-types`: `datainfo.rs` (`MatrixInfo`, `ElementType`), `value.rs` (`Value::Matrix`), decoding and encoding, and the spec-example tests in `tests/spec_examples.rs`, which no longer skip matrix.
- `crates/must-server`: none beyond the new `Value` variant. Parameters and command arguments of type matrix work like any other type. The model's `datainfo` checks accept it.
- `crates/must-ffi`, `crates/must-ffi-variant`:
  - direct mapping between `Value::Matrix` and the variant struct shape, without JSON;
  - `SECoP_V_create` for matrix datainfos;
  - `server/api.rs`: the two matrix functions.
- `include/SECoP-exports.h`: declarations of the two functions.
- `crates/must-ffi-ctests`: header checks, and C tests for a matrix parameter in callback and polling mode and for the two functions.
- Docs: `COMPATIBILITY.md`, the `must-types` crate docs (drop the "matrix is not supported" note).
- Line length: a 100×100 `<f8` matrix is about 107 kB as base64. A `change` with a matrix value is still bounded by the maximum line length for incoming messages (default 1 MiB), which this change leaves as it is.
