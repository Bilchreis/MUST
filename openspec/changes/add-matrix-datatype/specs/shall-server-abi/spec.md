# Spec Delta

## ADDED Requirements

### Requirement: Pushed matrix updates
The library SHALL export `SECoP_S_updateParameterMatrix(const char* szParameterName, const unsigned int* puLen, int iDims, const void* pData, int iDataSize, double dblTimestamp)`, a must addition. It pushes the matrix with `iDims` lengths `puLen` and `iDataSize` raw element bytes `pData` as the parameter's new value, like `SECoP_S_updateParameter2` and with the same timestamp rules. The library SHALL NOT keep `puLen` or `pData` after it returns.

#### Scenario: Push a matrix
- **WHEN** `my_node:mod1:image` has datainfo `{"type":"matrix","elementtype":"<f4","names":["x","y"],"maxlen":[100,100]}`, and `SECoP_S_updateParameterMatrix("my_node:mod1:image", len, 2, data, 24, NAN)` is called with `len` = {2, 3} and `data` the six floats 1–6
- **THEN** it returns `SECoP_S_SUCCESS`, and activated clients receive `update mod1:image [{"len": [2, 3], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"}, {"t": <now>}]`

#### Scenario: Not a matrix parameter
- **WHEN** `SECoP_S_updateParameterMatrix` names a parameter whose datainfo is not a matrix
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE` and nothing is sent

#### Scenario: Size does not fit
- **WHEN** `iDims` differs from the number of dimensions, or `iDataSize` from the product of `puLen` times the element size
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE` and nothing is sent

#### Scenario: Unknown parameter
- **WHEN** `SECoP_S_updateParameterMatrix` names a node, module or parameter that does not exist
- **THEN** it returns the same code `SECoP_S_updateParameter2` returns for that name

#### Scenario: Empty matrix
- **WHEN** it is called with all lengths 0, `iDataSize` 0 and `pData` NULL
- **THEN** it returns `SECoP_S_SUCCESS` and clients receive a matrix with an empty `blob`

### Requirement: Matrix answers to polled requests
The library SHALL export `SECoP_S_putCommandAnswerMatrix(unsigned long long llId, enum SECoP_S_error iErrorCode, const unsigned int* puLen, int iDims, const void* pData, int iDataSize, double dblTimestamp)`, a must addition. It answers a handed-out request like `SECoP_S_putCommandAnswer2`, with the matrix as the read value, the value after the change or the command result. With a negative `iErrorCode`, the matrix arguments SHALL be ignored and may be NULL.

#### Scenario: Answer a read
- **WHEN** a client reads `mod1:image`, and the application takes the request with `SECoP_S_getStoredCommand2` and answers with `SECoP_S_putCommandAnswerMatrix(id, SECoP_S_SUCCESS, len, 2, data, 24, NAN)`
- **THEN** the client receives `reply mod1:image [{"len": [2, 3], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"}, {...}]`

#### Scenario: Answer with an error
- **WHEN** the application answers with `SECoP_S_ERROR_COMM_FAILED` and NULL matrix arguments
- **THEN** the client receives `error_read mod1:image` with `CommunicationFailed`

#### Scenario: Request without a matrix value
- **WHEN** the request's accessible has no matrix value: a parameter that is not a matrix, or a command whose result is not one
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE`, and the request stays handed out and can still be answered

#### Scenario: Size does not fit
- **WHEN** `iDataSize` differs from the product of `puLen` times the element size
- **THEN** it returns `SECoP_S_ERROR_INVALID_VALUE`, and the request stays handed out

#### Scenario: Header declares the additions
- **WHEN** a C program includes `SECoP.h`
- **THEN** `SECoP_S_updateParameterMatrix` and `SECoP_S_putCommandAnswerMatrix` are declared, and the program compiles as C99 and C++11
