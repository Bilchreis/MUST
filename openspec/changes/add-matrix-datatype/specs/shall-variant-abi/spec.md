# Spec Delta

## ADDED Requirements

### Requirement: Matrix values as structs
The library SHALL represent a value of a `matrix` datainfo as a `SECoP_VT_STRUCT` handle with two members: `len` (`SECoP_VT_ARRAY_INTEGER`) and `blob` (`SECoP_VT_BLOB` holding the raw element bytes). `SECoP_V_toJSON` of such a handle SHALL give the SECoP matrix transport. No `SECoP_V_type` value or exported function is added.

#### Scenario: Created from a matrix datainfo
- **WHEN** `SECoP_V_create("{\"type\":\"matrix\",\"elementtype\":\"<f4\",\"names\":[\"x\",\"y\"],\"maxlen\":[100,100]}")` is called
- **THEN** it returns a handle whose position 0 is `SECoP_VT_STRUCT` with count 2, position 1 is `SECoP_VT_ARRAY_INTEGER` named `len` with count 2, and position 2 is `SECoP_VT_BLOB` named `blob`

#### Scenario: Parsed from transport JSON with a hint
- **WHEN** `SECoP_V_fromJSON("{\"len\":[2,3],\"blob\":\"AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA\"}", hint)` is called with a hint created from that datainfo
- **THEN** reading position 2 as a blob gives the 24 bytes of the six floats, and `SECoP_V_toJSON` gives the input JSON back

#### Scenario: Matrix parameter in polling mode
- **WHEN** a client sends `change mod1:image {"len":[1,1],"blob":"AACAPw=="}` for a writable matrix parameter and the application takes it with `SECoP_S_getStoredCommand`
- **THEN** the value handle is the struct shape above with `len` `[1, 1]` and the 4 bytes of `1.0f`

#### Scenario: Value that does not fit the datainfo
- **WHEN** a getter returns a matrix handle whose blob has fewer bytes than `len` requires
- **THEN** the client receives an error reply with `InternalError`, and no update is sent
