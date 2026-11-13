// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP values: decoding JSON against a [`DataInfo`](crate::DataInfo) and encoding back to JSON.
//!
//! Values arriving with `change` or `do` are decoded in [`Mode::Request`], which enforces
//! every limit of the datainfo (plus optional dynamic limits from `_min`/`_max`/`_limits`
//! parameters) and allows omitting optional struct members. Values in replies and updates
//! are decoded in [`Mode::Report`], which only checks the shape: a node may report a
//! measured value outside the declared range.

use std::fmt;

use indexmap::IndexMap;
use serde_json::Value as Json;
use thiserror::Error;

use crate::datainfo::json_to_i64;
use crate::error::ErrorClass;
use crate::path::{Path, PathSegment};

mod decode;
mod encode;
#[cfg(test)]
mod tests;

/// A typed SECoP value, matching some [`DataInfo`](crate::DataInfo).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// Value of a `double`.
    Double(f64),
    /// Transported integer of a `scaled`; see [`ScaledInfo::to_physical`](crate::datainfo::ScaledInfo::to_physical).
    Scaled(i64),
    /// Value of an `int`.
    Int(i64),
    /// Value of a `bool`.
    Bool(bool),
    /// Member value of an `enum`; see [`EnumInfo::name_of`](crate::datainfo::EnumInfo::name_of).
    Enum(i64),
    /// Value of a `string`.
    String(String),
    /// Bytes of a `blob`.
    Blob(Vec<u8>),
    /// Elements of an `array`.
    Array(Vec<Value>),
    /// Members of a `tuple`.
    Tuple(Vec<Value>),
    /// Members of a `struct`, in datainfo order. Optional members omitted in a request
    /// are absent.
    Struct(IndexMap<String, Value>),
    /// Value of a `matrix`: the length of each dimension and the raw element bytes, in
    /// transport order (first dimension varying fastest) and the byte order of the
    /// `elementtype`. See [`MatrixInfo`](crate::datainfo::MatrixInfo) for typed access to the elements.
    Matrix {
        /// Length of each dimension, in the order of [`MatrixInfo::names`](crate::datainfo::MatrixInfo::names).
        len: Vec<u32>,
        /// The elements as bytes, exactly `product(len) × element size` of them.
        data: Vec<u8>,
    },
}

impl Value {
    /// The value as a floating point number, for `double`, `int`, `scaled` (transported
    /// integer) and `enum`.
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Double(d) => Some(*d),
            Self::Int(i) | Self::Scaled(i) | Self::Enum(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// The value as an integer, for `int`, `scaled` (transported integer) and `enum`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) | Self::Scaled(i) | Self::Enum(i) => Some(*i),
            _ => None,
        }
    }

    /// The value of a `bool`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The text of a `string`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// The lengths and element bytes of a `matrix`.
    pub fn as_matrix(&self) -> Option<(&[u32], &[u8])> {
        match self {
            Self::Matrix { len, data } => Some((len, data)),
            _ => None,
        }
    }
}

/// How strictly to decode a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Input from `change` and `do`: all limits enforced, optional struct members may be
    /// omitted.
    Request,
    /// Values in replies and updates: only the shape is checked, all struct members are
    /// required.
    Report,
}

/// A value does not fit its datainfo.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub struct ValueError {
    /// SECoP error class to report.
    pub class: ErrorClass,
    /// Location of the offending element; empty for the top level.
    pub path: Path,
    /// Human readable description.
    pub message: String,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_root() {
            write!(f, "{}: {}", self.class, self.message)
        } else {
            write!(f, "{}: {}: {}", self.class, self.path, self.message)
        }
    }
}

impl ValueError {
    fn wrong_type(message: impl Into<String>) -> Self {
        Self {
            class: ErrorClass::WrongType,
            path: Path::root(),
            message: message.into(),
        }
    }

    fn range(message: impl Into<String>) -> Self {
        Self {
            class: ErrorClass::RangeError,
            path: Path::root(),
            message: message.into(),
        }
    }

    fn within(mut self, segment: PathSegment) -> Self {
        self.path = self.path.within(segment);
        self
    }
}

type Result<T> = std::result::Result<T, ValueError>;

/// Dynamic limits of a numeric parameter, i.e. the current values of its `_min`/`_max`
/// or `_limits` parameters, in the parameter's transport units (the transported integer
/// for `scaled`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DynLimits {
    /// Lower limit, if any.
    pub min: Option<f64>,
    /// Upper limit, if any.
    pub max: Option<f64>,
}

impl DynLimits {
    /// Limits from the value of a `_limits` parameter (a tuple of lower and upper limit).
    pub fn from_limits(limits: &Value) -> Option<Self> {
        match limits {
            Value::Tuple(pair) if pair.len() == 2 => Some(Self {
                min: pair[0].as_f64(),
                max: pair[1].as_f64(),
            }),
            _ => None,
        }
    }
}

/// Checks `value` against static and dynamic limits; the static ones take precedence.
fn check_range(
    value: f64,
    min: Option<f64>,
    max: Option<f64>,
    limits: Option<&DynLimits>,
    shown: impl fmt::Display,
) -> Result<()> {
    if let Some(min) = min
        && value < min
    {
        return Err(ValueError::range(format!(
            "{shown} is below the lower limit {min}"
        )));
    }
    if let Some(max) = max
        && value > max
    {
        return Err(ValueError::range(format!(
            "{shown} is above the upper limit {max}"
        )));
    }
    if let Some(DynLimits { min: Some(min), .. }) = limits
        && value < *min
    {
        return Err(ValueError::range(format!(
            "{shown} is below the dynamic lower limit {min}"
        )));
    }
    if let Some(DynLimits { max: Some(max), .. }) = limits
        && value > *max
    {
        return Err(ValueError::range(format!(
            "{shown} is above the dynamic upper limit {max}"
        )));
    }
    Ok(())
}

fn check_len(len: usize, min: u64, max: Option<u64>, unit: &str) -> Result<()> {
    let len = len as u64;
    if len < min {
        return Err(ValueError::range(format!(
            "{len} {unit}, at least {min} required"
        )));
    }
    if let Some(max) = max
        && len > max
    {
        return Err(ValueError::range(format!(
            "{len} {unit}, at most {max} allowed"
        )));
    }
    Ok(())
}

fn json_kind(json: &Json) -> &'static str {
    match json {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::String(_) => "a string",
        Json::Array(_) => "an array",
        Json::Object(_) => "an object",
    }
}

fn expected(what: &str, json: &Json) -> ValueError {
    ValueError::wrong_type(format!("expected {what}, got {}", json_kind(json)))
}

/// Integer from JSON: a JSON number with an integral value within `i64`.
fn decode_integer(json: &Json) -> Result<i64> {
    if let Some(i) = json_to_i64(json) {
        return Ok(i);
    }
    match json.as_f64() {
        Some(f) if f.fract() == 0.0 => Err(ValueError::range(format!(
            "{json} does not fit in a 64-bit integer"
        ))),
        Some(_) => Err(ValueError::wrong_type(format!(
            "expected an integer, got {json}"
        ))),
        None => Err(expected("an integer", json)),
    }
}
