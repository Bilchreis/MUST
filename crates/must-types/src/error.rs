// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP error classes and errors as sent in `error_*` replies.

use std::fmt;
use std::str::FromStr;

use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::value::ValueError;

macro_rules! error_classes {
    ($($(#[$doc:meta])* $name:ident,)*) => {
        /// The error classes of SECoP 2.0, sent as the first element of an error report.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum ErrorClass {
            $($(#[$doc])* $name,)*
        }

        impl ErrorClass {
            /// All error classes.
            pub const ALL: &[ErrorClass] = &[$(Self::$name,)*];

            /// The SECoP name of the class, e.g. `"NoSuchModule"`.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$name => stringify!($name),)*
                }
            }
        }

        impl FromStr for ErrorClass {
            type Err = UnknownErrorClass;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $(stringify!($name) => Ok(Self::$name),)*
                    _ => Err(UnknownErrorClass(s.to_owned())),
                }
            }
        }
    };
}

error_classes! {
    /// A malformed request or an unspecified message was sent.
    ProtocolError,
    /// The specified module does not exist.
    NoSuchModule,
    /// The specified parameter does not exist.
    NoSuchParameter,
    /// The specified command does not exist.
    NoSuchCommand,
    /// The requested write cannot be performed on a readonly value.
    ReadOnly,
    /// The requested check cannot be performed on the accessible.
    NotCheckable,
    /// The argument has the wrong type, or a struct lacks required members.
    WrongType,
    /// The argument is outside the range allowed by the datainfo.
    RangeError,
    /// The data part of the message is not valid JSON.
    BadJSON,
    /// The action is not (yet) implemented.
    NotImplemented,
    /// The connected hardware operates incorrectly or not at all.
    HardwareError,
    /// The command is already executing.
    CommandRunning,
    /// Communication with the hardware failed.
    CommunicationFailed,
    /// An initiated action took longer than the maximum allowed time.
    TimeoutError,
    /// The action cannot be performed while the module is busy.
    IsBusy,
    /// The action cannot be performed while the module is in error state.
    IsError,
    /// The action cannot be performed while the module is disabled.
    Disabled,
    /// The action cannot be performed at the moment.
    Impossible,
    /// The parameter cannot be read just now.
    ReadFailed,
    /// The value read from the hardware is out of sensor or calibration range.
    OutOfRange,
    /// Something that should never happen just happened.
    InternalError,
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A string is not the name of a SECoP error class.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown SECoP error class `{0}`")]
pub struct UnknownErrorClass(pub String);

/// An error to report to a SECoP client: class, human readable message and optional
/// implementation specific details.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{class}: {message}")]
pub struct SecopError {
    /// The SECoP error class.
    pub class: ErrorClass,
    /// A short human readable description.
    pub message: String,
    /// Extra information for the third element of the error report (the timestamp `t`
    /// is added when the report is sent).
    pub extra: Map<String, Json>,
}

impl SecopError {
    /// An error of `class` with `message` and no extra information.
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
            extra: Map::new(),
        }
    }

    /// Adds an entry to the extra information.
    #[must_use]
    pub fn with_extra(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.extra.insert(key.to_owned(), value.into());
        self
    }
}

impl From<ValueError> for SecopError {
    fn from(e: ValueError) -> Self {
        let message = if e.path.is_root() {
            e.message
        } else {
            format!("{}: {}", e.path, e.message)
        };
        Self::new(e.class, message)
    }
}
