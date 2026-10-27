# Spec Delta

## Purpose

Defines which names are valid SECoP identifiers and when two identifiers clash within a scope, so every part of must accepts and rejects the same names.

## ADDED Requirements

### Requirement: Identifier syntax
The library SHALL accept a module, accessible or property name only if it is 1 to 63 characters long, consists of ASCII letters, digits and underscores, and does not start with a digit. Names of postfixed parameters (e.g. `target_limits`) count in full.

#### Scenario: Valid identifiers
- **WHEN** the names `temp`, `T1`, `_custom` and `target_limits` are validated
- **THEN** each is accepted

#### Scenario: Leading digit
- **WHEN** the name `1temp` is validated
- **THEN** it is rejected with an error naming the invalid first character

#### Scenario: Invalid characters
- **WHEN** the names `temp-1`, `temp value` and `tempé` are validated
- **THEN** each is rejected

#### Scenario: Length limits
- **WHEN** an empty name or a name of 64 characters is validated
- **THEN** it is rejected, while a name of exactly 63 valid characters is accepted

#### Scenario: Postfix pushes a name over the limit
- **WHEN** the postfixed name formed from a 57-character parameter name and `_limits` is validated
- **THEN** it is rejected, because the full name has 64 characters

### Requirement: Custom identifiers
The library SHALL report whether an identifier is a custom name, which is the case exactly when it starts with an underscore.

#### Scenario: Custom name detection
- **WHEN** `_my_param` and `my_param` are checked
- **THEN** `_my_param` is reported as custom and `my_param` is not

### Requirement: Case-insensitive uniqueness within a scope
The library SHALL treat names as case sensitive when stored and compared, but SHALL reject adding a name to a scope that already holds a name equal to it after lowercasing. This applies to identifiers as well as struct member and enum member names.

#### Scenario: Clash differing only in case
- **WHEN** a scope holds `Temp` and `temp` is added
- **THEN** the addition is rejected with an error naming the existing `Temp`

#### Scenario: Case is preserved
- **WHEN** `Temp` is added to an empty scope and then looked up
- **THEN** it is found as `Temp`, and a lookup of `temp` does not find it
