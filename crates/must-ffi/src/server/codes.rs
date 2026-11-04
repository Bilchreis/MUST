// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_S_error` and `SECoP_S_action` codes, their descriptions, and their
//! mapping to SECoP 2.0 error classes; and the user error codes of
//! `SECoP_S_registerUserError`.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_int};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use must_types::{ErrorClass, SecopError, validate_identifier};

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
    /// A MUST addition, not in SHALL: the dry run of a `change` or `do`.
    pub const CHECK: c_int = 4;
}

/// SHALL's description of a code, or `<name>: <text>` of a registered user error, as
/// returned by `SECoP_S_getErrorString`.
pub(crate) fn description(code: c_int) -> &'static CStr {
    if let Some(user) = user_error(code) {
        return user.description;
    }
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

/// The SECoP 2.0 error class for one of SHALL's error codes returned by the application
/// (the design's mapping table).
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

/// The error sent to a client for an error code returned by the application of
/// `context` for `accessible`. A user error code maps to its class and text only for
/// the context that registered it; any other is an unknown code.
pub(crate) fn to_secop(code: c_int, context: &str, accessible: &str) -> SecopError {
    match user_error(code) {
        Some(user) if user.context == context => {
            SecopError::custom(user.name, format!("{accessible}: {}", user.text))
        }
        Some(_) => SecopError::new(
            ErrorClass::InternalError,
            format!("{accessible}: {}", unknown(code)),
        ),
        None => {
            let text = description(code).to_string_lossy();
            SecopError::new(error_class(code), format!("{accessible}: {text}"))
        }
    }
}

/// SHALL's description of an unknown code.
fn unknown(code: c_int) -> &'static str {
    if code >= 0 {
        "INVALID: this is an unknown SECoP warning"
    } else {
        "INVALID: this is an unknown SECoP error"
    }
}

// --- user error codes

/// The first code of `SECoP_S_registerUserError`; later ones count down from here, far
/// below SHALL's own codes.
pub(crate) const FIRST_USER_ERROR: c_int = -1000;

/// A registered user error.
#[derive(Clone)]
pub(crate) struct UserError {
    /// The context that registered it, the only one that uses it.
    pub context: String,
    /// The SECoP error class name.
    pub name: String,
    pub text: String,
    /// `<name>: <text>`, valid until the process ends.
    description: &'static CStr,
}

/// The registered user errors. Its own lock, never held together with the state's.
struct Registry {
    by_code: HashMap<c_int, UserError>,
    next: c_int,
    /// Every description handed out, which callers of `SECoP_S_getErrorString` may keep:
    /// allocated once per distinct text and never freed, like SHALL's static strings.
    descriptions: HashMap<String, &'static CStr>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    Mutex::new(Registry {
        by_code: HashMap::new(),
        next: FIRST_USER_ERROR,
        descriptions: HashMap::new(),
    })
});

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Registry {
    fn description(&mut self, name: &str, text: &str) -> &'static CStr {
        let full = format!("{name}: {text}");
        self.descriptions.entry(full).or_insert_with_key(|full| {
            // a NUL in the text ends it there
            let bytes = full.split('\0').next().unwrap_or_default();
            let c = CString::new(bytes).unwrap_or_default();
            Box::leak(c.into_boxed_c_str())
        })
    }
}

/// The user error registered with `code`, by any context, if any.
pub(crate) fn user_error(code: c_int) -> Option<UserError> {
    if code > FIRST_USER_ERROR {
        return None;
    }
    registry().by_code.get(&code).cloned()
}

/// Registers the error class `name` with `text` for `context` (which the caller has
/// checked exists) and returns its code (at most [`FIRST_USER_ERROR`]), or an error
/// code. A name the context has already registered keeps its code and gets the new text;
/// other contexts' names do not matter.
pub(crate) fn register_user_error(context: &str, name: Option<&str>, text: &str) -> c_int {
    let Some(name) = name.filter(|n| validate_identifier(n).is_ok()) else {
        return err::INVALID_NAME;
    };
    // a SECoP 2.0 class only in its exact spelling
    if ErrorClass::ALL
        .iter()
        .any(|c| c.as_str().eq_ignore_ascii_case(name) && c.as_str() != name)
    {
        return err::NAME_ALREADY_USED;
    }
    let mut registry = registry();
    let known = registry
        .by_code
        .iter()
        .find(|(_, u)| u.context == context && u.name.eq_ignore_ascii_case(name))
        .map(|(code, u)| (*code, u.name == name));
    let code = match known {
        Some((_, false)) => return err::NAME_ALREADY_USED,
        Some((code, true)) => code,
        None => {
            let code = registry.next;
            let Some(next) = code.checked_sub(1) else {
                return err::NO_MEMORY;
            };
            registry.next = next;
            code
        }
    };
    let description = registry.description(name, text);
    registry.by_code.insert(
        code,
        UserError {
            context: context.to_owned(),
            name: name.to_owned(),
            text: text.to_owned(),
            description,
        },
    );
    code
}

/// Unregisters a code that `context` registered with [`register_user_error`]; it is
/// never handed out again. `SECoP_S_ERROR_INVALID_VALUE` for any other code.
pub(crate) fn unregister_user_error(context: &str, code: c_int) -> c_int {
    let mut registry = registry();
    if registry
        .by_code
        .get(&code)
        .is_none_or(|u| u.context != context)
    {
        return err::INVALID_VALUE;
    }
    registry.by_code.remove(&code);
    err::SUCCESS
}

/// Removes every user error code of `context`.
pub(crate) fn remove_context_user_errors(context: &str) {
    registry().by_code.retain(|_, u| u.context != context);
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
        // the design's mapping table
        let table = [
            (err::INVALID_MODULE, ErrorClass::NoSuchModule),
            (err::INVALID_PARAMETER, ErrorClass::NoSuchParameter),
            (err::INVALID_COMMAND, ErrorClass::NoSuchCommand),
            (err::READONLY, ErrorClass::ReadOnly),
            (err::INVALID_VALUE, ErrorClass::WrongType),
            (err::OUT_OF_RANGE, ErrorClass::RangeError),
            (err::NOT_IMPLEMENTED, ErrorClass::NotImplemented),
            (err::TIMEOUT, ErrorClass::TimeoutError),
            (err::COMM_FAILED, ErrorClass::CommunicationFailed),
            (err::COMMAND_FAILED, ErrorClass::HardwareError),
            (err::COMMAND_RUNNING, ErrorClass::CommandRunning),
            (err::IS_BUSY, ErrorClass::IsBusy),
            (err::IS_ERROR, ErrorClass::IsError),
            (err::DISABLED, ErrorClass::Disabled),
            (err::SYNTAX, ErrorClass::ProtocolError),
            (err::INTERNAL, ErrorClass::InternalError),
        ];
        for (code, class) in table {
            assert_eq!(error_class(code), class, "code {code}");
        }
        for code in -40..0 {
            if !table.iter().any(|(c, _)| *c == code) {
                assert_eq!(error_class(code), ErrorClass::InternalError, "code {code}");
            }
        }
        let error = to_secop(err::IS_BUSY, "ctx", "target");
        assert!(error.to_string().contains("target"));
    }

    // The registry is global and shared with other tests: each test uses contexts of its
    // own.

    #[test]
    fn user_error_codes() {
        const CTX: &str = "codes-tests-a";
        let code = register_user_error(CTX, Some("_Overheated"), "coil too hot");
        assert!(code <= FIRST_USER_ERROR, "{code}");
        assert_eq!(description(code), c"_Overheated: coil too hot");
        let error = to_secop(code, CTX, "target");
        assert_eq!(error.class_name(), "_Overheated");
        assert_eq!(error.message, "target: coil too hot");

        // again: the same code, the new text
        assert_eq!(
            register_user_error(CTX, Some("_Overheated"), "coil too hot!"),
            code
        );
        let kept = description(code);
        assert_eq!(kept, c"_Overheated: coil too hot!");
        assert_eq!(to_secop(code, CTX, "x").message, "x: coil too hot!");

        // removed by one unregister, its description still readable, never reused
        assert_eq!(unregister_user_error(CTX, code), err::SUCCESS);
        assert_eq!(kept, c"_Overheated: coil too hot!");
        assert_eq!(to_secop(code, CTX, "x").class_name(), "InternalError");
        assert!(description(code).to_bytes().starts_with(b"INVALID"));
        assert_eq!(unregister_user_error(CTX, code), err::INVALID_VALUE);
        let again = register_user_error(CTX, Some("_Overheated"), "coil too hot");
        assert!(again < code, "{again} {code}");
        assert_eq!(unregister_user_error(CTX, again), err::SUCCESS);
    }

    #[test]
    fn user_error_names() {
        const CTX: &str = "codes-tests-b";
        let code = register_user_error(CTX, Some("_Overheated"), "");
        assert_eq!(
            register_user_error(CTX, Some("_overheated"), ""),
            err::NAME_ALREADY_USED
        );
        assert_eq!(
            register_user_error(CTX, Some("hardwareerror"), ""),
            err::NAME_ALREADY_USED
        );
        for bad in [None, Some(""), Some("1st error"), Some("over-heated")] {
            assert_eq!(
                register_user_error(CTX, bad, ""),
                err::INVALID_NAME,
                "{bad:?}"
            );
        }
        assert_eq!(unregister_user_error(CTX, code), err::SUCCESS);

        // a SECoP 2.0 class with a text of its own
        let code = register_user_error(CTX, Some("HardwareError"), "sensor disconnected");
        assert!(code <= FIRST_USER_ERROR);
        let error = to_secop(code, CTX, "value");
        assert_eq!(error.class, ErrorClass::HardwareError);
        assert_eq!(error.class_name(), "HardwareError");
        assert_eq!(error.message, "value: sensor disconnected");
        assert_eq!(unregister_user_error(CTX, code), err::SUCCESS);
    }

    #[test]
    fn user_errors_belong_to_their_context() {
        const A: &str = "codes-tests-c";
        const B: &str = "codes-tests-d";
        // the same name in two contexts: two codes, two texts
        let a = register_user_error(A, Some("_Overheated"), "coil A too hot");
        let b = register_user_error(B, Some("_Overheated"), "coil B too hot");
        assert!(a <= FIRST_USER_ERROR && b <= FIRST_USER_ERROR && a != b);
        assert_eq!(to_secop(a, A, "t").message, "t: coil A too hot");
        assert_eq!(to_secop(b, B, "t").message, "t: coil B too hot");
        // within a context, another case still clashes
        assert_eq!(
            register_user_error(B, Some("_OVERHEATED"), ""),
            err::NAME_ALREADY_USED
        );

        // a code of another context is unknown there
        let foreign = to_secop(b, A, "t");
        assert_eq!(foreign.class_name(), "InternalError");
        assert!(foreign.message.contains("unknown"), "{}", foreign.message);
        assert_eq!(unregister_user_error(A, b), err::INVALID_VALUE);
        assert_eq!(to_secop(b, B, "t").class_name(), "_Overheated");

        // removed with the context, the other context's code stays
        remove_context_user_errors(A);
        assert!(user_error(a).is_none());
        assert_eq!(unregister_user_error(B, b), err::SUCCESS);
    }

    #[test]
    fn unknown_codes_are_not_unregistered() {
        for code in [
            err::SUCCESS,
            err::IS_BUSY,
            err::INTERNAL,
            5,
            -31,
            c_int::MIN,
        ] {
            assert_eq!(
                unregister_user_error("codes-tests-e", code),
                err::INVALID_VALUE,
                "{code}"
            );
        }
    }
}
