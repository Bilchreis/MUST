# Spec Delta

## Purpose

Models SECoP 2.0 `datainfo` descriptions: parsing them from JSON, checking them for consistency, exposing their properties with spec defaults, and serializing them back to JSON.

## ADDED Requirements

### Requirement: Supported data types
The library SHALL parse a `datainfo` JSON object whose `type` is one of `double`, `scaled`, `int`, `bool`, `enum`, `string`, `blob`, `array`, `tuple`, `struct` or `command`, and SHALL reject any other or missing `type`.

#### Scenario: Known type
- **WHEN** `{"type": "double", "min": 0, "max": 100, "unit": "K"}` is parsed
- **THEN** a double datainfo with min 0, max 100 and unit `K` is returned

#### Scenario: Unknown type
- **WHEN** `{"type": "float"}` is parsed
- **THEN** parsing fails with an error naming the unknown type `float`

#### Scenario: Matrix not supported yet
- **WHEN** `{"type": "matrix", "elementtype": "<f4", "names": ["x"], "maxlen": [100]}` is parsed
- **THEN** parsing fails with an error stating that the type `matrix` is not supported

#### Scenario: Not an object
- **WHEN** `"double"` or `[]` is parsed as a datainfo
- **THEN** parsing fails

### Requirement: Mandatory data properties
The library SHALL reject a datainfo that lacks a mandatory property of its type: `scale`, `min`, `max` for scaled; `min`, `max` for int; `members` for enum, tuple and struct; `maxbytes` for blob; `members` and `maxlen` for array.

#### Scenario: Missing int limits
- **WHEN** `{"type": "int", "min": 0}` is parsed
- **THEN** parsing fails with an error naming the missing property `max`

#### Scenario: Missing blob maxbytes
- **WHEN** `{"type": "blob"}` is parsed
- **THEN** parsing fails with an error naming the missing property `maxbytes`

### Requirement: Property types
The library SHALL reject a datainfo whose known properties have the wrong JSON type, including non-integer limits for int and scaled, non-integer lengths, and a non-string `unit` or `fmtstr`.

#### Scenario: Fractional int limit
- **WHEN** `{"type": "int", "min": 0.5, "max": 10}` is parsed
- **THEN** parsing fails

#### Scenario: Negative length
- **WHEN** `{"type": "string", "maxchars": -1}` is parsed
- **THEN** parsing fails

### Requirement: Consistent limits
The library SHALL reject a datainfo whose lower bound exceeds its upper bound (`min`/`max`, `minchars`/`maxchars`, `minbytes`/`maxbytes`, `minlen`/`maxlen`), and SHALL accept equal bounds.

#### Scenario: Inverted range
- **WHEN** `{"type": "double", "min": 10, "max": 0}` is parsed
- **THEN** parsing fails

#### Scenario: Single allowed value
- **WHEN** `{"type": "int", "min": 5, "max": 5}` is parsed
- **THEN** parsing succeeds

### Requirement: Scale factor
The library SHALL require the `scale` of a scaled datainfo to be a finite number greater than zero.

#### Scenario: Zero scale
- **WHEN** `{"type": "scaled", "scale": 0, "min": 0, "max": 10}` is parsed
- **THEN** parsing fails

### Requirement: Format string syntax
The library SHALL accept a `fmtstr` only if it matches `%.` followed by an optional digit 1-9, one digit, and one of `e`, `f` or `g`.

#### Scenario: Valid format strings
- **WHEN** datainfos with `fmtstr` `%.3f`, `%.12g` and `%.0e` are parsed
- **THEN** each is accepted

#### Scenario: Invalid format strings
- **WHEN** datainfos with `fmtstr` `%5.3f`, `%d` or `%.3` are parsed
- **THEN** each is rejected

### Requirement: Enum members
The library SHALL require enum `members` to be an object mapping non-empty names to integers, with names unique case-insensitively and values unique, and SHALL keep the members in the given order. Enum names are not required to follow the identifier syntax.

#### Scenario: Duplicate value
- **WHEN** `{"type": "enum", "members": {"on": 1, "active": 1}}` is parsed
- **THEN** parsing fails with an error naming the duplicate value 1

#### Scenario: Name lookup in both directions
- **WHEN** `{"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}` is parsed
- **THEN** the name `BUSY` maps to 300 and the value 100 maps to `IDLE`

### Requirement: Structured member types
The library SHALL parse the nested datainfos of array `members`, tuple `members` (a list) and struct `members` (an object with non-empty member names, unique case-insensitively), and SHALL reject a nested datainfo of type `command`.

#### Scenario: Nested datainfo
- **WHEN** `{"type": "array", "maxlen": 10, "members": {"type": "int", "min": 0, "max": 9}}` is parsed
- **THEN** an array datainfo with an int element datainfo is returned

#### Scenario: Struct member names clash
- **WHEN** `{"type": "struct", "members": {"X": {"type": "double"}, "x": {"type": "double"}}}` is parsed
- **THEN** parsing fails with an error naming the clashing member `x`

#### Scenario: Nested command
- **WHEN** `{"type": "tuple", "members": [{"type": "command"}]}` is parsed
- **THEN** parsing fails

### Requirement: Optional struct members
The library SHALL accept a struct `optional` list only if every entry names a member of that struct.

#### Scenario: Unknown optional member
- **WHEN** `{"type": "struct", "members": {"x": {"type": "double"}}, "optional": ["y"]}` is parsed
- **THEN** parsing fails with an error naming `y`

### Requirement: Command argument and result
The library SHALL parse a command's optional `argument` and `result` as either `null`/absent (no value) or a non-command datainfo.

#### Scenario: Command without argument
- **WHEN** `{"type": "command", "result": {"type": "bool"}}` is parsed
- **THEN** the command has no argument and a bool result

### Requirement: Spec defaults
The library SHALL report the SECoP default for an optional property that was not given: for double, `fmtstr` `%.6g`, `absolute_resolution` 0 and `relative_resolution` 1.2e-7; for scaled, `absolute_resolution` equal to `scale`, `relative_resolution` 1.2e-7 and `fmtstr` `%.<n>f` with n = max(0, -floor(log10(scale))); 0 for `minchars`, `minbytes` and `minlen`; false for `isUTF8`.

#### Scenario: Scaled default format
- **WHEN** `{"type": "scaled", "scale": 0.01, "min": 0, "max": 1000}` is parsed
- **THEN** its effective `fmtstr` is `%.2f` and its effective `absolute_resolution` is 0.01

### Requirement: Unknown properties are preserved
The library SHALL accept properties it does not know (custom or from newer spec versions) without error and SHALL keep them for serialization.

#### Scenario: Custom property round trip
- **WHEN** `{"type": "bool", "_note": "x"}` is parsed and serialized again
- **THEN** the output contains `"_note": "x"`

### Requirement: Serialization round trip
The library SHALL serialize a datainfo to JSON that parses back to an equal datainfo, emitting only properties that were given (no defaults added) and keeping enum and struct member order.

#### Scenario: Round trip keeps order
- **WHEN** `{"type": "struct", "members": {"y": {"type": "double"}, "x": {"type": "bool"}}}` is parsed and serialized
- **THEN** the output lists member `y` before `x` and parses to an equal datainfo

### Requirement: Datainfos built in code
The library SHALL apply the same validation to a datainfo constructed in code as to one parsed from JSON.

#### Scenario: Invalid datainfo built in code
- **WHEN** an int datainfo with min 10 and max 0 is constructed in code
- **THEN** construction fails with the same error as parsing the equivalent JSON
