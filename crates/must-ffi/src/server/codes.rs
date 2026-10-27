// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_S_error` and `SECoP_S_action` codes, their descriptions, and their
//! mapping to SECoP 2.0 error classes.

use std::ffi::{CStr, c_int};

use must_types::{ErrorClass, SecopError};

/// `enum SECoP_S_error` of `SECoP-types.h`.
#[allow(dead_code, reason = "the complete list of SHALL's codes")]
pub(crate) mod err {
    use std::ffi::c_int;

    pub const SUCCESS: c_int = 0;
    pub const WARNING_NO_DESCRIPTION: c_int = 1;
    pub const WARNING_CUSTOM_PROPERTY: c_int = 2;
    pub const WARNING_BUFFER_TOO_SMALL: c_int = 3;
    pub const WARNING_MISS_PROPERTIES: c_int = 4;
    pub const UNKNOWN_COMMAND: c_int = -1;
    pub const INVALID_NAME: c_int = -2;
    pub const INVALID_NODE: c_int = -3;
    pub const INVALID_MODULE: c_int = -4;
    pub const INVALID_PARAMETER: c_int = -5;
    pub const INVALID_PROPERTY: c_int = -6;
    pub const INVALID_COMMAND: c_int = -7;
    pub const NOT_IMPLEMENTED: c_int = -8;
    pub const READONLY: c_int = -9;
    pub const NO_DATA: c_int = -10;
    pub const NO_MEMORY: c_int = -11;
    pub const NOT_INITIALIZED: c_int = -12;
    pub const INVALID_VALUE: c_int = -13;
    pub const MISSING_MANDATORY: c_int = -14;
    pub const NO_SETTER_GETTER: c_int = -15;
    pub const NO_SETTER: c_int = -16;
    pub const NO_GETTER: c_int = -17;
    pub const NAME_ALREADY_USED: c_int = -18;
    pub const TIMEOUT: c_int = -19;
    pub const WRONG_BITNESS: c_int = -20;
    pub const COMMAND_FAILED: c_int = -21;
    pub const COMMAND_RUNNING: c_int = -22;
    pub const COMM_FAILED: c_int = -23;
    pub const IS_BUSY: c_int = -24;
    pub const IS_ERROR: c_int = -25;
    pub const DISABLED: c_int = -26;
    pub const SYNTAX: c_int = -27;
    pub const INTERNAL: c_int = -28;
    pub const OUT_OF_RANGE: c_int = -29;
    pub const INVALID_CONTEXT_ID: c_int = -30;
}

/// `enum SECoP_S_action` of `SECoP-types.h`.
pub(crate) mod action {
    use std::ffi::c_int;

    pub const READ: c_int = 1;
    pub const CHANGE: c_int = 2;
    pub const DO: c_int = 3;
}

/// SHALL's description of a code, as returned by `SECoP_S_getErrorString`.
pub(crate) fn description(code: c_int) -> &'static CStr {
    match code {
        err::SUCCESS => c"SECoP_S_SUCCESS: success",
        err::WARNING_NO_DESCRIPTION => c"SECoP_S_WARNING_NO_DESCRIPTION: no description was given",
        err::WARNING_CUSTOM_PROPERTY => {
            c"SECoP_S_WARNING_CUSTOM_PROPERTY: custom name of property should be prefixed with an underscore"
        }
        err::WARNING_BUFFER_TOO_SMALL => {
            c"SECoP_S_WARNING_BUFFER_TOO_SMALL: input buffer is too small, some information is missing (call again with greater buffer)"
        }
        err::WARNING_MISS_PROPERTIES => c"SECoP_S_WARNING_MISS_PROPERTIES: missing optional properties",
        err::UNKNOWN_COMMAND => c"SECoP_S_ERROR_UNKNOWN_COMMAND: invalid SECoP command",
        err::INVALID_NAME => c"SECoP_S_ERROR_INVALID_NAME: invalid SECoP name",
        err::INVALID_NODE => c"SECoP_S_ERROR_INVALID_NODE: invalid node reference",
        err::INVALID_MODULE => c"SECoP_S_ERROR_INVALID_MODULE: invalid module reference",
        err::INVALID_PARAMETER => c"SECoP_S_ERROR_INVALID_PARAMETER: invalid parameter reference",
        err::INVALID_PROPERTY => c"SECoP_S_ERROR_INVALID_PROPERTY: invalid property reference",
        err::INVALID_COMMAND => c"SECoP_S_ERROR_INVALID_COMMAND: invalid command (reference)",
        err::NOT_IMPLEMENTED => c"SECoP_S_ERROR_NOT_IMPLEMENTED: function call is not implemented",
        err::READONLY => c"SECoP_S_ERROR_READONLY: element is read only",
        err::NO_DATA => c"SECoP_S_ERROR_NO_DATA: function call cannot return any data",
        err::NO_MEMORY => c"SECoP_S_ERROR_NO_MEMORY: memory allocation problem",
        err::NOT_INITIALIZED => c"SECoP_S_ERROR_NOT_INITIALIZED: call SECoP_S_initLibrary() first",
        err::INVALID_VALUE => c"SECoP_S_ERROR_INVALID_VALUE: datainfo or value was invalid",
        err::MISSING_MANDATORY => c"SECoP_S_ERROR_MISSING_MANDATORY: an expected property was empty",
        err::NO_SETTER_GETTER => {
            c"SECoP_S_ERROR_NO_SETTER_GETTER: missing a pointer to getter and setter function"
        }
        err::NO_SETTER => c"SECoP_S_ERROR_NO_SETTER: missing a pointer to setter function",
        err::NO_GETTER => c"SECoP_S_ERROR_NO_GETTER: missing a pointer to getter function",
        err::NAME_ALREADY_USED => c"SECoP_S_ERROR_NAME_ALREADY_USED: the name is used before",
        err::TIMEOUT => c"SECoP_S_ERROR_TIMEOUT: parameter or command timeout",
        err::WRONG_BITNESS => c"SECoP_S_ERROR_WRONG_BITNESS: unsupported platform",
        err::COMMAND_FAILED => c"SECoP_S_ERROR_COMMAND_FAILED: SECoP error \"CommandFailed\"",
        err::COMMAND_RUNNING => c"SECoP_S_ERROR_COMMAND_RUNNING: SECoP error \"CommandRunning\"",
        err::COMM_FAILED => c"SECoP_S_ERROR_COMM_FAILED: SECoP error \"CommunicationFailed\"",
        err::IS_BUSY => c"SECoP_S_ERROR_IS_BUSY: SECoP error \"IsBusy\"",
        err::IS_ERROR => c"SECoP_S_ERROR_IS_ERROR: SECoP error \"IsError\"",
        err::DISABLED => c"SECoP_S_ERROR_DISABLED: SECoP error \"Disabled\"",
        err::SYNTAX => c"SECoP_S_ERROR_SYNTAX: SECoP error \"SyntaxError\"",
        err::INTERNAL => c"SECoP_S_ERROR_INTERNAL: SECoP error \"InternalError\"",
        err::OUT_OF_RANGE => c"SECoP_S_ERROR_OUT_OF_RANGE: value is not inside the allowed range",
        err::INVALID_CONTEXT_ID => c"SECoP_S_ERROR_INVALID_CONTEXT_ID: invalid context ID",
        c if c >= 0 => c"INVALID: this is an unknown SECoP warning",
        _ => c"INVALID: this is an unknown SECoP error",
    }
}

/// The SECoP 2.0 error class for an error code returned by the application (the design's
/// mapping table).
pub(crate) fn error_class(code: c_int) -> ErrorClass {
    match code {
        err::INVALID_MODULE => ErrorClass::NoSuchModule,
        err::INVALID_PARAMETER => ErrorClass::NoSuchParameter,
        err::INVALID_COMMAND => ErrorClass::NoSuchCommand,
        err::READONLY => ErrorClass::ReadOnly,
        err::INVALID_VALUE => ErrorClass::WrongType,
        err::OUT_OF_RANGE => ErrorClass::RangeError,
        err::NOT_IMPLEMENTED => ErrorClass::NotImplemented,
        err::TIMEOUT => ErrorClass::TimeoutError,
        err::COMM_FAILED => ErrorClass::CommunicationFailed,
        err::COMMAND_FAILED => ErrorClass::HardwareError,
        err::COMMAND_RUNNING => ErrorClass::CommandRunning,
        err::IS_BUSY => ErrorClass::IsBusy,
        err::IS_ERROR => ErrorClass::IsError,
        err::DISABLED => ErrorClass::Disabled,
        err::SYNTAX => ErrorClass::ProtocolError,
        _ => ErrorClass::InternalError,
    }
}

/// The error sent to a client for an error code returned by the application for
/// `accessible`.
pub(crate) fn to_secop(code: c_int, accessible: &str) -> SecopError {
    let text = description(code).to_string_lossy();
    SecopError::new(error_class(code), format!("{accessible}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_description() {
        for code in -30..=4 {
            assert!(
                !description(code).to_bytes().starts_with(b"INVALID"),
                "code {code}"
            );
        }
        assert!(description(-31).to_bytes().starts_with(b"INVALID"));
        assert!(description(5).to_bytes().starts_with(b"INVALID"));
    }

    #[test]
    fn mapped_classes() {
        assert_eq!(error_class(err::IS_BUSY), ErrorClass::IsBusy);
        assert_eq!(error_class(err::COMMAND_FAILED), ErrorClass::HardwareError);
        assert_eq!(error_class(err::NO_MEMORY), ErrorClass::InternalError);
    }
}
