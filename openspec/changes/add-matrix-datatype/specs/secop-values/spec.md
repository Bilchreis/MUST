# Spec Delta

## ADDED Requirements

### Requirement: Matrix values
The library SHALL decode `matrix` from a JSON object with exactly the members `len` (one non-negative integer per dimension) and `blob` (padded single-line base64 of the elements, first dimension varying fastest). Any other shape, or a byte count other than the product of `len` times the element size, is a `WrongType` error. A `len` entry above its `maxlen` is a `RangeError` in request mode. Encoding SHALL give the same form.

#### Scenario: Example from the specification
- **WHEN** `{"len": [2, 3], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"}` is decoded against `{"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}`
- **THEN** the result has lengths `[2, 3]` and 24 bytes, whose elements read as `<f4` in order are 1, 2, 3, 4, 5, 6

#### Scenario: Blob size does not fit the lengths
- **WHEN** `{"len": [2, 3], "blob": "AACAPw=="}` is decoded against the same datainfo
- **THEN** decoding fails with a `WrongType` error naming `blob`

#### Scenario: Wrong number of dimensions
- **WHEN** `{"len": [6], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"}` is decoded against the same datainfo
- **THEN** decoding fails with a `WrongType` error naming `len`

#### Scenario: Dimension beyond maxlen
- **WHEN** a matrix with `len` `[101, 1]` and 404 bytes is decoded in request mode against the same datainfo
- **THEN** decoding fails with a `RangeError` whose path points at `len` index 0

#### Scenario: Empty matrix
- **WHEN** `{"len": [0, 0], "blob": ""}` is decoded against the same datainfo
- **THEN** decoding succeeds with no elements

#### Scenario: Unknown member
- **WHEN** `{"len": [1, 1], "blob": "AACAPw==", "t": 0}` is decoded against the same datainfo
- **THEN** decoding fails with a `WrongType` error naming `t`

#### Scenario: Round trip
- **WHEN** a matrix value is decoded, encoded and decoded again
- **THEN** the second result equals the first and the encoded `blob` is identical to the input
