# Spec Delta

## MODIFIED Requirements

### Requirement: Mandatory data properties
The library SHALL reject a datainfo that lacks a mandatory property of its type: `scale`, `min`, `max` for scaled; `min`, `max` for int; `members` for enum, tuple and struct; `maxbytes` for blob; `members` and `maxlen` for array; `names`, `maxlen` and `elementtype` for matrix.

#### Scenario: Missing int limits
- **WHEN** `{"type": "int", "min": 0}` is parsed
- **THEN** parsing fails with an error naming the missing property `max`

#### Scenario: Missing blob maxbytes
- **WHEN** `{"type": "blob"}` is parsed
- **THEN** parsing fails with an error naming the missing property `maxbytes`

#### Scenario: Missing matrix elementtype
- **WHEN** `{"type": "matrix", "names": ["x"], "maxlen": [10]}` is parsed
- **THEN** parsing fails with an error naming the missing property `elementtype`

## ADDED Requirements

### Requirement: Recognized data types
The library SHALL parse a `datainfo` JSON object whose `type` is one of `double`, `scaled`, `int`, `bool`, `enum`, `string`, `blob`, `array`, `tuple`, `struct`, `matrix` or `command`, and SHALL reject any other or missing `type`.

#### Scenario: Known type
- **WHEN** `{"type": "double", "min": 0, "max": 100, "unit": "K"}` is parsed
- **THEN** a double datainfo with min 0, max 100 and unit `K` is returned

#### Scenario: Unknown type
- **WHEN** `{"type": "float"}` is parsed
- **THEN** parsing fails with an error naming the unknown type `float`

#### Scenario: Matrix
- **WHEN** `{"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}` is parsed
- **THEN** a matrix datainfo with two dimensions `x` and `y`, each at most 100 long, and little-endian 4-byte floats is returned

#### Scenario: Not an object
- **WHEN** `"double"` or `[]` is parsed as a datainfo
- **THEN** parsing fails

### Requirement: Matrix properties
The library SHALL reject a matrix datainfo unless `names` is a non-empty list of distinct strings, `maxlen` lists one positive integer per name, and `elementtype` is `<` or `>`, then `i`, `u` or `f`, then a byte count of 1, 2, 4 or 8, except `f1`. A `compression` property SHALL be rejected as unsupported.

#### Scenario: Big-endian unsigned
- **WHEN** `{"type": "matrix", "elementtype": ">u2", "names": ["ch"], "maxlen": [4096]}` is parsed
- **THEN** parsing succeeds with big-endian 2-byte unsigned elements

#### Scenario: Invalid element type
- **WHEN** a matrix datainfo with `elementtype` `"<f1"`, `"<x4"`, `"f4"` or `"<i3"` is parsed
- **THEN** parsing fails with an error naming the property `elementtype`

#### Scenario: Length count differs from names
- **WHEN** `{"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100]}` is parsed
- **THEN** parsing fails with an error naming `maxlen`

#### Scenario: Duplicate dimension name
- **WHEN** a matrix datainfo with `names` `["x", "x"]` is parsed
- **THEN** parsing fails with an error naming `names`

#### Scenario: Compression
- **WHEN** a matrix datainfo with `"compression": "zlib"` is parsed
- **THEN** parsing fails with an error stating that `compression` is not supported

#### Scenario: Round trip
- **WHEN** a valid matrix datainfo is parsed and serialized again
- **THEN** the JSON has the same `type`, `names`, `maxlen` and `elementtype`

## REMOVED Requirements

### Requirement: Supported data types
**Reason**: Its scenario "Matrix not supported yet" no longer holds. "Recognized data types" replaces it with `matrix` in the list and a matrix scenario.
**Migration**: None. Every datainfo accepted before is still accepted.
