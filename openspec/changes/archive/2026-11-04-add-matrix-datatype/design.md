# Design

## Context

See proposal.md for the motivation and the specs for the required behavior.

**Today's code.**
- `must-types/src/datainfo.rs` maps `"matrix"` to `DataInfoErrorKind::UnsupportedType`. `DataInfo` has one variant per type. `decode`/`encode` dispatch on it, in request or report mode.
- `Value` (`must-types/src/value.rs`) has no matrix variant. `Value::Blob` already handles base64 decoding and encoding.
- The C ABI passes values as JSON internally. A getter's or `putCommandAnswer2`'s handle is exported to JSON and decoded against the accessible's `DataInfo` in report mode (`must-ffi/src/server/handler.rs`, `Answer::reading`). A decoding failure becomes an `InternalError` reply. Requests travel the other way: `Value` → JSON → `variant::Node::from_json` with the accessible's datainfo as hint.
- `variant::Node` (`must-ffi/src/variant.rs`) has kinds for scalars, scalar arrays, bytes, strings, structs, tuples and arrays. `type_json`/`to_datainfo` derive a datainfo back from the node. `SECoP_S_addProperty("datainfo", handle)` relies on this.

## Goals / Non-Goals

**Goals:**
- A matrix accessible works end to end in Rust and over the C ABI. Its elements are never copied or converted on the way through the node.
- No new `SECoP_V_type` value. SHALL-built hosts and the LabVIEW VIs can use matrices with the existing calls; the two matrix functions are an optional faster path.
- A pushed matrix update reads the host's bytes once and base64-encodes them once, whichever entry point the host uses.

**Non-Goals:**
- `compression`, and the side-channel transport that SECoP mentions as a possible future extension.
- Element access through `SECoP_V_get*`/`modify*`. Hosts read and write the blob as bytes.
- Any change to the incoming line length limit.

## Decisions

### `Value::Matrix { len: Vec<u32>, data: Vec<u8> }` holds raw bytes
The value keeps the bytes in transport order and endianness. Typed access goes through helpers on `MatrixInfo`, such as `element_f64(&data, index)` and `from_f64s(lens, &[f64])`, which know the element type.
- *Alternative:* store decoded elements as an enum of typed vectors (`Vec<f32>`, ...). Rejected: every update would pay for converting the bytes in and out. Most nodes just pass device buffers through. The bytes also have to stay bit-exact, for NaN payloads and for big-endian data.

### Validation mirrors array and blob
- **Shape (`WrongType`), in both modes:** the object has exactly `len` and `blob`; `len` has one entry per name; the base64 is valid; the byte count equals the product of `len` times the element size.
- **Limits (`RangeError`), in request mode only:** a `len` entry above its `maxlen`. This is how `array` treats `maxlen`.

An overflowing product of the lengths is a `WrongType` error. Use checked multiplication, since `usize` is 32 bits on i686.

### Matrix in the C ABI is a plain struct node
`variant::Node::from_json` with a matrix hint, and `SECoP_V_create` with a matrix datainfo, build a `Kind::Struct` with members `len` (a `ScalarArray` of int) and `blob` (`Kind::Bytes`). The node keeps the original matrix datainfo in a side field. `type_json`/`to_datainfo` then return `{"type": "matrix", ...}` rather than the struct, so `SECoP_S_addProperty("datainfo", SECoP_V_create(matrix))` publishes a matrix.

The JSON export of this struct is exactly the SECoP transport, so `SECoP_V_toJSON` and `SECoP_S_updateParameter2`/`putCommandAnswer2` need no special case. The JSON path is not used for matrices inside must, though (see the next decision).
- *Alternative:* a new `SECoP_VT_MATRIX`. Rejected by the user: it widens the ABI beyond SHALL's.
- *Alternative:* JSON only. Rejected: hosts would have to base64-encode themselves.

### Matrices bypass JSON inside must-ffi
For the other types, values cross between handles and `Value` as JSON (`export_value` → `decode`, and `encode` → `from_json`). For a matrix that would base64-encode the data, decode it, and later encode it again for the wire. Instead, `variant::Node` gets `to_value_direct(&DataInfo)` and `from_value_direct(&Value, &DataInfo)`. For a matrix they move or clone the `len` vector and the byte buffer, and check the shape with the same function `MatrixInfo` uses for decoding. For every other type they fall back to the JSON path, so nothing else changes behavior.

These functions are used wherever a handle meets a `Value`:
- `Node::to_value` (`updateParameter`)
- getter and command results in `handler.rs`
- `putCommandAnswer`
- requests handed to setters and the polling queue

`Answer` then carries an `AnswerValue` (JSON text, a handle node, or a matrix), so `putCommandAnswer` with a handle and `putCommandAnswerMatrix` do not go through JSON either. `updateParameter2`/`putCommandAnswer2` with transport JSON still decode base64 once. That is inherent to their input.
- *Alternative:* add a matrix fast path only to the new functions. Rejected: getters in callback mode are the common way to serve a parameter, and they would keep the double encoding.

### Flat matrix functions: lengths and bytes as plain pointers
`SECoP_S_updateParameterMatrix` and `SECoP_S_putCommandAnswerMatrix` take `const unsigned int* puLen, int iDims, const void* pData, int iDataSize`, the plain C types a LabVIEW Call Library node passes for U32 and U8 arrays ("Array Data Pointer"). Sizes are `int`, like SHALL's other sizes, which limits a matrix to 2 GiB; the line limits and memory reach their own limits long before that.
- **`updateParameterMatrix`:** checks the name as `updateParameter2` does (same codes), then that the parameter is a matrix and that the shape fits. It copies `pData` once into a `Vec<u8>` and stores the value through the same `update` path. The `update` line's base64 is then the only encoding.
- **`putCommandAnswerMatrix`:** looks up the handed-out request's accessible and its value datainfo (parameter datainfo, or command `result`) before taking the request out of `handed_out`. A request without a matrix value, or a shape that does not fit, returns `INVALID_VALUE` and leaves the request handed out, so the host can still answer it correctly. With a negative error code, the matrix arguments are not read.
- Neither function takes a sigma. SECoP's `e` qualifier for a matrix would itself need to be a matrix, and no host is known to send one. `putCommandAnswer2` stays available for that case.
- *Alternative:* a zero-copy pointer that must keeps until the line is written. Rejected: the host would have to know when must is done with its buffer. LabVIEW cannot express that, and one copy is cheap next to base64.

### Modifying a matrix handle
`SECoP_V_getString`/`SECoP_V_modifyString` (with an explicit byte count, as for any `SECoP_VT_BLOB`) on position 2, and `SECoP_V_modifyArrayLength`/`SECoP_V_modifyInteger` on position 1, work as for any struct member. They check against the member types only. Whether `len` and the blob agree is checked when the value reaches the node (getter result, `updateParameter`, `putCommandAnswer`), which reports `InternalError` or `SECoP_S_ERROR_INVALID_VALUE`. This keeps SHALL's member-at-a-time editing possible: resize `len`, then replace the blob.

## Risks / Trade-offs

- [The direct conversion must give exactly what the JSON path gives, or matrices would behave differently depending on the entry point] → Both use the same shape check in must-types. A test pushes the same matrix through `updateParameter` (handle), `updateParameter2` (JSON) and `updateParameterMatrix`, and compares the update lines byte for byte.
- [Large matrices make large lines: every update is one base64 line, sent to every activated client] → The existing slow-client handling applies (queue limits and disconnection). The docs note the size cost (4/3 of the bytes).
- [`SECoP_V_getInfo` reports a matrix as `SECoP_VT_STRUCT`, so a host cannot tell a matrix from a real struct with these members by type code alone] → Hosts know their own datainfo. `COMPATIBILITY.md` documents the shape.
- [Half floats (`f2`) cannot be read by Rust helpers without a dependency] → The `f64` helper is offered for `f4`/`f8` and all integer types only. `f2` data passes through as bytes. This is documented on the helper.
