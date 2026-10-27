# Spec Delta

## Purpose

Converts SECoP values between JSON and typed values according to a `datainfo`, checking them and classifying failures as SECoP `WrongType` or `RangeError` so a node or client can report them on the wire.

## ADDED Requirements

### Requirement: Validation modes
The library SHALL decode values in one of two modes: request mode (for `change` and `do` input), which enforces all limits and allows omitting optional struct members; and report mode (for `reply`, `update` and similar), which checks only the value's shape and requires all struct members.

#### Scenario: Out-of-range value in report mode
- **WHEN** `150.0` is decoded against `{"type": "double", "min": 0, "max": 100}` in report mode
- **THEN** decoding succeeds

#### Scenario: Out-of-range value in request mode
- **WHEN** `150.0` is decoded against the same datainfo in request mode
- **THEN** decoding fails with a `RangeError`

### Requirement: Error classification and location
The library SHALL classify each decoding failure as `WrongType` (the JSON shape does not fit the datainfo) or `RangeError` (the shape fits but a limit is violated), and SHALL report the path to the failing element and a human readable message.

#### Scenario: Wrong JSON type
- **WHEN** `"hot"` is decoded against `{"type": "double"}`
- **THEN** decoding fails with a `WrongType` error

#### Scenario: Nested failure path
- **WHEN** `[1, 2, 99]` is decoded in request mode against an array of `{"type": "int", "min": 0, "max": 9}`
- **THEN** decoding fails with a `RangeError` whose path points at index 2

### Requirement: Numeric values
The library SHALL decode `double` from any JSON number, and `int`, `scaled` and `enum` from JSON numbers with an integral value that fits in a signed 64-bit integer; limits are inclusive.

#### Scenario: Integer accepted for double
- **WHEN** `5` is decoded against `{"type": "double"}`
- **THEN** the double value 5.0 is returned

#### Scenario: Fraction rejected for int
- **WHEN** `2.5` is decoded against `{"type": "int", "min": 0, "max": 10}`
- **THEN** decoding fails with a `WrongType` error

#### Scenario: Integral float accepted for int
- **WHEN** `3.0` is decoded against `{"type": "int", "min": 0, "max": 10}`
- **THEN** the integer value 3 is returned

#### Scenario: Inclusive limit
- **WHEN** `10` is decoded in request mode against `{"type": "int", "min": 0, "max": 10}`
- **THEN** decoding succeeds

### Requirement: Scaled conversion
The library SHALL convert between the transported integer of a scaled value and its physical value (integer × `scale`), rounding a physical value to the nearest transported integer.

#### Scenario: Physical value of scaled
- **WHEN** `1255` is decoded against `{"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}`
- **THEN** its physical value is 125.5

#### Scenario: Physical to transported
- **WHEN** the physical value 12.34 is converted for scale 0.1
- **THEN** the transported integer is 123

### Requirement: Dynamic limits
The library SHALL accept, when decoding a `double`, `scaled` or `int` value in request mode, an optional dynamic lower and upper limit (the current values of the `_min`/`_max` or `_limits` parameters, in the parent's transport units) and reject a value outside them with a `RangeError`, in addition to the static `min`/`max`.

#### Scenario: Value outside dynamic limits
- **WHEN** `250.0` is decoded in request mode against `{"type": "double", "min": 0, "max": 300}` with dynamic limits 10 to 200
- **THEN** decoding fails with a `RangeError` that names the dynamic upper limit 200

#### Scenario: Value inside dynamic limits
- **WHEN** `150.0` is decoded in request mode with the same limits
- **THEN** decoding succeeds

#### Scenario: Static limits still apply
- **WHEN** `-5.0` is decoded in request mode against the same datainfo with only a dynamic upper limit of 200
- **THEN** decoding fails with a `RangeError` naming the static lower limit 0

#### Scenario: Dynamic limits ignored in report mode
- **WHEN** `250.0` is decoded in report mode with dynamic limits 10 to 200
- **THEN** decoding succeeds

#### Scenario: Scaled dynamic limits use transported integers
- **WHEN** `1500` is decoded in request mode against `{"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}` with a dynamic upper limit of 1000
- **THEN** decoding fails with a `RangeError`

### Requirement: Enum values
The library SHALL accept an enum value only if it equals one of the member values, in both modes.

#### Scenario: Unknown member value
- **WHEN** `250` is decoded against `{"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}`
- **THEN** decoding fails with a `RangeError`

### Requirement: Boolean values
The library SHALL decode `bool` only from JSON `true` or `false`.

#### Scenario: Number for bool
- **WHEN** `1` is decoded against `{"type": "bool"}`
- **THEN** decoding fails with a `WrongType` error

### Requirement: String values
The library SHALL decode `string` from a JSON string, counting length in Unicode code points against `minchars`/`maxchars` and, unless `isUTF8` is true, rejecting characters outside 7-bit ASCII as a `RangeError` in request mode.

#### Scenario: Length counted in characters
- **WHEN** `"äöü"` is decoded in request mode against `{"type": "string", "maxchars": 3, "isUTF8": true}`
- **THEN** decoding succeeds

#### Scenario: Non-ASCII without isUTF8
- **WHEN** `"ä"` is decoded in request mode against `{"type": "string"}`
- **THEN** decoding fails with a `RangeError`

### Requirement: Blob values
The library SHALL decode `blob` from a JSON string holding standard base64 (RFC 4648, with padding), checking the decoded byte count against `minbytes`/`maxbytes`, and SHALL encode blobs as single-line padded base64.

#### Scenario: Blob decoding
- **WHEN** `"U0VDb1A="` is decoded against `{"type": "blob", "maxbytes": 8}`
- **THEN** the bytes of the ASCII string `SECoP` are returned

#### Scenario: Invalid base64
- **WHEN** `"not base64!"` is decoded against a blob datainfo
- **THEN** decoding fails with a `WrongType` error

### Requirement: Array and tuple values
The library SHALL decode `array` and `tuple` from JSON arrays, decoding each element against its member datainfo; an array length outside `minlen`/`maxlen` is a `RangeError`, and a tuple with a different number of elements than members is a `WrongType` error.

#### Scenario: Array too long
- **WHEN** a 4-element JSON array is decoded in request mode against an array with `maxlen` 3
- **THEN** decoding fails with a `RangeError`

#### Scenario: Tuple arity
- **WHEN** `[300]` is decoded against a tuple with two members
- **THEN** decoding fails with a `WrongType` error

### Requirement: Struct values
The library SHALL decode `struct` from a JSON object, decoding each member against its datainfo, rejecting unknown members and missing non-optional members as `WrongType`, and allowing optional members to be absent only in request mode.

#### Scenario: Optional member omitted in a request
- **WHEN** `{"x": 1.0, "y": 2.0}` is decoded in request mode against a struct with members `x`, `y`, `t` where `t` is optional
- **THEN** decoding succeeds and the result marks `t` as absent

#### Scenario: Optional member omitted in a report
- **WHEN** the same value is decoded in report mode
- **THEN** decoding fails with a `WrongType` error naming `t`

#### Scenario: Unknown member
- **WHEN** `{"x": 1.0, "z": 0}` is decoded against a struct with members `x` and `y`
- **THEN** decoding fails with a `WrongType` error naming `z`

### Requirement: Command arguments and results
The library SHALL decode a command argument against the command's `argument` datainfo, and SHALL accept only JSON `null` or no value when the command has no argument; results are handled the same way with `result`.

#### Scenario: Unexpected argument
- **WHEN** `5` is decoded as the argument of `{"type": "command"}`
- **THEN** decoding fails with a `WrongType` error

### Requirement: Encoding values
The library SHALL encode a typed value to JSON according to its datainfo such that decoding the output yields an equal value, and SHALL refuse to encode a non-finite double or a value whose shape does not fit the datainfo.

#### Scenario: Round trip
- **WHEN** a struct value is decoded in report mode, encoded, and decoded again
- **THEN** the second result equals the first

#### Scenario: Non-finite double
- **WHEN** a double value of NaN is encoded
- **THEN** encoding fails, since JSON cannot carry NaN
