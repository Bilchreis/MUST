# shall-variant-abi Specification

## Purpose

Provides SHALL's `SECoP_V_*` C functions so that C and LabVIEW code written for SHALL's VariantLib can create, inspect, modify, convert and free SECoP values with must.

## Requirements

### Requirement: Exported variant functions
The library SHALL export, with SHALL's exact names and signatures, every C function of SHALL's `SECoP-Variant.h` except the C++-only `SECoP_V_printString` and `SECoP_V_printStream`.

#### Scenario: Symbol list
- **WHEN** the exported symbols of `SHALLvariant64` are listed
- **THEN** they contain every `SECoP_V_*` C function of SHALL's header, such as `SECoP_V_create`, `SECoP_V_fromJSON`, `SECoP_V_toJSON`, `SECoP_V_getInfo` and `SECoP_V_destroy`

### Requirement: Enum values
The library SHALL use SHALL's numeric values for `SECoP_V_type` and `SECoP_V_compareResult`.

#### Scenario: Type codes
- **WHEN** `SECoP_V_getInfo` reports the type of a scaled value and of a struct
- **THEN** it reports `SECoP_VT_SCALED` (5) and `SECoP_VT_STRUCT` (30)

### Requirement: Opaque handles
The library SHALL represent values as opaque `CSECoPbaseType*` handles that carry both the datainfo and the value, are created by the `SECoP_V_create*`, `SECoP_V_fromJSON` and `SECoP_V_copy` functions, and are freed only by `SECoP_V_destroy`, which sets the caller's pointer to NULL.

#### Scenario: Create and destroy
- **WHEN** a handle is created with `SECoP_V_createSimpleDouble(1.5)` and destroyed with `SECoP_V_destroy(&p)`
- **THEN** `p` is NULL afterwards and no memory is leaked

#### Scenario: Invalid description
- **WHEN** `SECoP_V_create("{\"type\":\"nope\"}")` is called
- **THEN** it returns NULL

### Requirement: JSON conversion
The library SHALL convert between handles and JSON as SHALL does: `SECoP_V_fromJSON` parses a value, using an optional hint handle for its datainfo, and `SECoP_V_toJSON` writes into a caller buffer and returns the needed size.

#### Scenario: Buffer too small
- **WHEN** `SECoP_V_toJSON` is called for a value whose JSON needs 20 bytes with a 10-byte buffer
- **THEN** it returns the needed size and does not overflow the buffer, so a second call with a large enough buffer gets the full JSON

#### Scenario: JSON with datainfo hint
- **WHEN** `SECoP_V_fromJSON("1255", hint)` is called with a hint created from `{"type":"scaled","scale":0.1,"min":0,"max":2500}`
- **THEN** the result is a scaled value whose physical value is 125.5

### Requirement: Positional access
The library SHALL implement SHALL's position-based getters and modifiers (`SECoP_V_get*`, `SECoP_V_modify*`, `SECoP_V_getInfo`, enum, struct-name and additional-property accessors) with the same addressing of nested items by `uPosition` and the same return conventions.

#### Scenario: Array of doubles is one position
- **WHEN** `SECoP_V_getInfo` is called at position 0 of the array value `[1.0, 2.5]` and then `SECoP_V_getDouble(p, 0, 1, &d)`
- **THEN** getInfo reports `SECoP_VT_ARRAY_DOUBLE` with count 2, and getDouble returns 1 with `d` = 2.5

#### Scenario: Nested positions
- **WHEN** `SECoP_V_getInfo` walks a struct `{"a": 1.0, "b": "x"}` from position 0 upward
- **THEN** position 0 is the struct itself, position 1 the double named `a`, position 2 the string named `b`, and position 3 returns 0 (end of data)

#### Scenario: Out-of-range position
- **WHEN** a getter is called with a position past the last item
- **THEN** it returns 0 and leaves the output unchanged

### Requirement: Comparison
The library SHALL compare two handles like SHALL's `SECoP_V_compare`, returning `SECoP_VC_EQUAL`, `SECoP_VC_SIMILAR_VALUE`, `SECoP_VC_DIFF_VALUE` or `SECoP_VC_DIFF_TYPE`.

#### Scenario: Different types
- **WHEN** a double handle and a string handle are compared
- **THEN** the result is `SECoP_VC_DIFF_TYPE`

### Requirement: Robust against NULL
The library SHALL treat NULL handle and buffer arguments as errors reported through the function's normal failure result, never as a crash.

#### Scenario: NULL handle
- **WHEN** `SECoP_V_toJSON(NULL, buf, 100)` is called
- **THEN** it returns a negative value and does not crash
