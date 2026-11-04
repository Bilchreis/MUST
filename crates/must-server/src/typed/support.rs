// SPDX-License-Identifier: LGPL-3.0-or-later

//! Helpers for the code the module macros generate. Their typed signatures make a
//! method whose types do not match its declaration a type error at that method.

use must_types::{DataInfo, ErrorClass, SecopError, Value};
use serde_json::{Map, Value as Json};

use super::{Command, Param, SecopType};

pub use indexmap::IndexMap;

/// Converts a read or write method's result to the parameter's SECoP value.
pub fn encode_as<T: SecopType>(
    _param: &Param<T>,
    value: &T,
    info: &DataInfo,
) -> Result<Value, SecopError> {
    value.to_value(info)
}

/// Converts a value from a `change` to the parameter's Rust type.
pub fn decode_as<T: SecopType>(
    _param: &Param<T>,
    value: &Value,
    info: &DataInfo,
) -> Result<T, SecopError> {
    T::from_value(value, info)
}

/// Converts a command's argument to its Rust type.
pub fn command_argument<A: SecopType, R>(
    _command: &Command<A, R>,
    argument: Option<&Value>,
    info: Option<&DataInfo>,
) -> Result<A, SecopError> {
    match (argument, info) {
        (Some(value), Some(info)) => A::from_value(value, info),
        _ => Err(SecopError::new(
            ErrorClass::InternalError,
            "the command's argument is missing",
        )),
    }
}

/// Converts a command's result to its SECoP value.
pub fn command_result<A, R: SecopType>(
    _command: &Command<A, R>,
    result: &R,
    info: Option<&DataInfo>,
) -> Result<Option<Value>, SecopError> {
    match info {
        Some(info) => result.to_value(info).map(Some),
        None => Ok(None),
    }
}

/// The error for an accessible the generated dispatch does not know.
pub fn unknown(what: &str, name: &str) -> SecopError {
    SecopError::new(
        ErrorClass::InternalError,
        format!("`{name}` is not a {what} of this module"),
    )
}

/// A `struct` datainfo from its members and the names of the optional ones.
pub fn struct_datainfo(
    members: Vec<(&str, Result<DataInfo, String>)>,
    optional: &[&str],
) -> Result<DataInfo, String> {
    let mut map = Map::new();
    for (name, info) in members {
        let info = info.map_err(|e| format!("member `{name}`: {e}"))?;
        map.insert(name.to_owned(), info.to_json());
    }
    let mut json = Map::new();
    json.insert("type".into(), "struct".into());
    json.insert("members".into(), Json::Object(map));
    if !optional.is_empty() {
        json.insert(
            "optional".into(),
            Json::Array(optional.iter().map(|n| Json::from(*n)).collect()),
        );
    }
    DataInfo::from_json(&Json::Object(json)).map_err(|e| e.to_string())
}

/// The member datainfo of a struct member: `info` of the member's `DataInfo`.
pub fn struct_member<'i>(info: &'i DataInfo, name: &str) -> Result<&'i DataInfo, SecopError> {
    match info {
        DataInfo::Struct(s) => s.members().get(name).ok_or_else(|| {
            SecopError::new(
                ErrorClass::InternalError,
                format!("the struct datainfo has no member `{name}`"),
            )
        }),
        _ => Err(SecopError::new(
            ErrorClass::InternalError,
            format!("expected a struct datainfo, got {}", info.to_json()),
        )),
    }
}
