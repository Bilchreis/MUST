// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP values: decoding JSON against a [`DataInfo`] and encoding back to JSON.
//!
//! Values arriving with `change` or `do` are decoded in [`Mode::Request`], which enforces
//! every limit of the datainfo (plus optional dynamic limits from `_min`/`_max`/`_limits`
//! parameters) and allows omitting optional struct members. Values in replies and updates
//! are decoded in [`Mode::Report`], which only checks the shape: a node may report a
//! measured value outside the declared range.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use indexmap::IndexMap;
use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::datainfo::{
    ArrayInfo, BlobInfo, CommandInfo, DataInfo, DoubleInfo, EnumInfo, IntInfo, ScaledInfo,
    StringInfo, StructInfo, TupleInfo, json_to_i64,
};
use crate::error::ErrorClass;
use crate::path::{Path, PathSegment};

/// A typed SECoP value, matching some [`DataInfo`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// Value of a `double`.
    Double(f64),
    /// Transported integer of a `scaled`; see [`ScaledInfo::to_physical`].
    Scaled(i64),
    /// Value of an `int`.
    Int(i64),
    /// Value of a `bool`.
    Bool(bool),
    /// Member value of an `enum`; see [`EnumInfo::name_of`].
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

// ---------------------------------------------------------------------------------------
// decoding

impl DataInfo {
    /// Decodes and checks a JSON value.
    pub fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        self.decode_with(json, mode, None)
    }

    /// Decodes a value from `change` or `do` in request mode, also checking `limits` for
    /// `double`, `scaled` and `int`.
    pub fn decode_request(&self, json: &Json, limits: Option<&DynLimits>) -> Result<Value> {
        self.decode_with(json, Mode::Request, limits)
    }

    fn decode_with(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        // dynamic limits only matter for requests
        let limits = limits.filter(|_| mode == Mode::Request);
        match self {
            Self::Double(i) => i.decode(json, mode, limits),
            Self::Scaled(i) => i.decode(json, mode, limits),
            Self::Int(i) => i.decode(json, mode, limits),
            Self::Bool(_) => match json {
                Json::Bool(b) => Ok(Value::Bool(*b)),
                _ => Err(expected("true or false", json)),
            },
            Self::Enum(i) => i.decode(json),
            Self::String(i) => i.decode(json, mode),
            Self::Blob(i) => i.decode(json, mode),
            Self::Array(i) => i.decode(json, mode),
            Self::Tuple(i) => i.decode(json, mode),
            Self::Struct(i) => i.decode(json, mode),
            Self::Command(_) => Err(ValueError::wrong_type(
                "a command has no value; decode its argument or result",
            )),
        }
    }
}

impl DoubleInfo {
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let value = json.as_f64().ok_or_else(|| expected("a number", json))?;
        if mode == Mode::Request {
            check_range(value, self.min(), self.max(), limits, value)?;
        }
        Ok(Value::Double(value))
    }
}

impl ScaledInfo {
    #[allow(clippy::cast_precision_loss)]
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let raw = decode_integer(json)?;
        if mode == Mode::Request {
            let (min, max) = (self.min() as f64, self.max() as f64);
            check_range(raw as f64, Some(min), Some(max), limits, raw)?;
        }
        Ok(Value::Scaled(raw))
    }
}

impl IntInfo {
    #[allow(clippy::cast_precision_loss)]
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let value = decode_integer(json)?;
        if mode == Mode::Request {
            let (min, max) = (self.min() as f64, self.max() as f64);
            check_range(value as f64, Some(min), Some(max), limits, value)?;
        }
        Ok(Value::Int(value))
    }
}

impl EnumInfo {
    fn decode(&self, json: &Json) -> Result<Value> {
        let value = decode_integer(json)?;
        if self.name_of(value).is_none() {
            return Err(ValueError::range(format!(
                "{value} is not a member of the enum"
            )));
        }
        Ok(Value::Enum(value))
    }
}

impl StringInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::String(s) = json else {
            return Err(expected("a string", json));
        };
        if mode == Mode::Request {
            check_len(
                s.chars().count(),
                self.minchars(),
                self.maxchars(),
                "characters",
            )?;
            if !self.is_utf8()
                && let Some(c) = s.chars().find(|c| !c.is_ascii())
            {
                return Err(ValueError::range(format!(
                    "non-ASCII character `{c}`, but the string is not declared as UTF-8"
                )));
            }
        }
        Ok(Value::String(s.clone()))
    }
}

impl BlobInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::String(s) = json else {
            return Err(expected("a base64 string", json));
        };
        let bytes = BASE64
            .decode(s)
            .map_err(|e| ValueError::wrong_type(format!("invalid base64: {e}")))?;
        if mode == Mode::Request {
            check_len(bytes.len(), self.minbytes(), Some(self.maxbytes()), "bytes")?;
        }
        Ok(Value::Blob(bytes))
    }
}

impl ArrayInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Array(items) = json else {
            return Err(expected("an array", json));
        };
        if mode == Mode::Request {
            check_len(items.len(), self.minlen(), Some(self.maxlen()), "elements")?;
        }
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                self.members()
                    .decode(item, mode)
                    .map_err(|e| e.within(PathSegment::Index(i)))
            })
            .collect::<Result<_>>()
            .map(Value::Array)
    }
}

impl TupleInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Array(items) = json else {
            return Err(expected("an array", json));
        };
        if items.len() != self.members().len() {
            return Err(ValueError::wrong_type(format!(
                "expected {} tuple members, got {}",
                self.members().len(),
                items.len()
            )));
        }
        self.members()
            .iter()
            .zip(items)
            .enumerate()
            .map(|(i, (info, item))| {
                info.decode(item, mode)
                    .map_err(|e| e.within(PathSegment::Index(i)))
            })
            .collect::<Result<_>>()
            .map(Value::Tuple)
    }
}

impl StructInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Object(items) = json else {
            return Err(expected("an object", json));
        };
        if let Some(unknown) = items.keys().find(|k| !self.members().contains_key(*k)) {
            return Err(ValueError::wrong_type(format!(
                "unknown struct member `{unknown}`"
            )));
        }
        let mut members = IndexMap::new();
        for (name, info) in self.members() {
            match items.get(name) {
                Some(item) => {
                    let value = info
                        .decode(item, mode)
                        .map_err(|e| e.within(PathSegment::Member(name.clone())))?;
                    members.insert(name.clone(), value);
                }
                None if mode == Mode::Request && self.is_optional(name) => {}
                None => {
                    return Err(ValueError::wrong_type(format!(
                        "missing struct member `{name}`"
                    )));
                }
            }
        }
        Ok(Value::Struct(members))
    }
}

impl CommandInfo {
    /// Decodes the argument of a `do` request; `json` is `None` when the message has no
    /// data part. Returns `None` for a command without argument.
    pub fn decode_argument(&self, json: Option<&Json>) -> Result<Option<Value>> {
        decode_optional(self.argument(), json, Mode::Request)
            .map_err(|e| e.within(PathSegment::Argument))
    }

    /// Decodes the result in a `done` reply. Returns `None` for a command without result.
    pub fn decode_result(&self, json: &Json) -> Result<Option<Value>> {
        decode_optional(self.result(), Some(json), Mode::Report)
            .map_err(|e| e.within(PathSegment::Result))
    }

    /// Encodes the argument for a `do` request (`null` for a command without argument).
    pub fn encode_argument(&self, value: Option<&Value>) -> Result<Json> {
        encode_optional(self.argument(), value).map_err(|e| e.within(PathSegment::Argument))
    }

    /// Encodes the result for a `done` reply (`null` for a command without result).
    pub fn encode_result(&self, value: Option<&Value>) -> Result<Json> {
        encode_optional(self.result(), value).map_err(|e| e.within(PathSegment::Result))
    }
}

fn decode_optional(
    info: Option<&DataInfo>,
    json: Option<&Json>,
    mode: Mode,
) -> Result<Option<Value>> {
    match (info, json) {
        (None, None | Some(Json::Null)) => Ok(None),
        (None, Some(json)) => Err(ValueError::wrong_type(format!(
            "expected no value or null, got {}",
            json_kind(json)
        ))),
        (Some(_), None) => Err(ValueError::wrong_type("a value is required")),
        (Some(info), Some(json)) => info.decode(json, mode).map(Some),
    }
}

fn encode_optional(info: Option<&DataInfo>, value: Option<&Value>) -> Result<Json> {
    match (info, value) {
        (None, None) => Ok(Json::Null),
        (None, Some(_)) => Err(ValueError::wrong_type(
            "the command takes or returns no value",
        )),
        (Some(_), None) => Err(ValueError::wrong_type("a value is required")),
        (Some(info), Some(value)) => info.encode(value),
    }
}

// ---------------------------------------------------------------------------------------
// encoding

impl DataInfo {
    /// Encodes a value to JSON. Fails if the value does not match the datainfo's shape,
    /// is a non-finite double, is not a member of an enum, or lacks struct members.
    /// Limits are not checked, as a node may report values outside its declared range.
    pub fn encode(&self, value: &Value) -> Result<Json> {
        match (self, value) {
            (Self::Double(_), Value::Double(d)) => serde_json::Number::from_f64(*d)
                .map(Json::Number)
                .ok_or_else(|| ValueError::wrong_type(format!("{d} cannot be sent as JSON"))),
            (Self::Scaled(_), Value::Scaled(i)) | (Self::Int(_), Value::Int(i)) => Ok((*i).into()),
            (Self::Bool(_), Value::Bool(b)) => Ok((*b).into()),
            (Self::Enum(info), Value::Enum(i)) => match info.name_of(*i) {
                Some(_) => Ok((*i).into()),
                None => Err(ValueError::wrong_type(format!(
                    "{i} is not a member of the enum"
                ))),
            },
            (Self::String(_), Value::String(s)) => Ok(s.as_str().into()),
            (Self::Blob(_), Value::Blob(bytes)) => Ok(BASE64.encode(bytes).into()),
            (Self::Array(info), Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    info.members()
                        .encode(item)
                        .map_err(|e| e.within(PathSegment::Index(i)))
                })
                .collect::<Result<Vec<_>>>()
                .map(Json::Array),
            (Self::Tuple(info), Value::Tuple(items)) => {
                if items.len() != info.members().len() {
                    return Err(ValueError::wrong_type(format!(
                        "expected {} tuple members, got {}",
                        info.members().len(),
                        items.len()
                    )));
                }
                info.members()
                    .iter()
                    .zip(items)
                    .enumerate()
                    .map(|(i, (member, item))| {
                        member
                            .encode(item)
                            .map_err(|e| e.within(PathSegment::Index(i)))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Json::Array)
            }
            (Self::Struct(info), Value::Struct(items)) => {
                if let Some(unknown) = items.keys().find(|k| !info.members().contains_key(*k)) {
                    return Err(ValueError::wrong_type(format!(
                        "unknown struct member `{unknown}`"
                    )));
                }
                let mut out = Map::new();
                for (name, member) in info.members() {
                    let item = items.get(name).ok_or_else(|| {
                        ValueError::wrong_type(format!("missing struct member `{name}`"))
                    })?;
                    let json = member
                        .encode(item)
                        .map_err(|e| e.within(PathSegment::Member(name.clone())))?;
                    out.insert(name.clone(), json);
                }
                Ok(Json::Object(out))
            }
            (Self::Command(_), _) => Err(ValueError::wrong_type(
                "a command has no value; encode its argument or result",
            )),
            (info, value) => Err(ValueError::wrong_type(format!(
                "a {} value does not fit a `{}` datainfo",
                value_kind(value),
                info.type_name()
            ))),
        }
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Double(_) => "double",
        Value::Scaled(_) => "scaled",
        Value::Int(_) => "int",
        Value::Bool(_) => "bool",
        Value::Enum(_) => "enum",
        Value::String(_) => "string",
        Value::Blob(_) => "blob",
        Value::Array(_) => "array",
        Value::Tuple(_) => "tuple",
        Value::Struct(_) => "struct",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(clippy::needless_pass_by_value)]
    fn info(json: Json) -> DataInfo {
        DataInfo::from_json(&json).unwrap()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn class_of(result: Result<Value>) -> ErrorClass {
        result.expect_err("should fail").class
    }

    // --- 4.1 errors and paths

    #[test]
    fn error_display() {
        let e = ValueError::range("too big").within(PathSegment::Member("x".into()));
        let e = e.within(PathSegment::Index(2));
        assert_eq!(e.to_string(), "RangeError: [2].x: too big");
        assert_eq!(ValueError::wrong_type("bad").to_string(), "WrongType: bad");
        assert_eq!(ErrorClass::WrongType.as_str(), "WrongType");
    }

    // --- 4.2 numeric, enum, bool

    #[test]
    fn report_mode_ignores_range() {
        let d = info(json!({"type": "double", "min": 0, "max": 100}));
        assert_eq!(
            d.decode(&json!(150.0), Mode::Report),
            Ok(Value::Double(150.0))
        );
        assert_eq!(
            class_of(d.decode(&json!(150.0), Mode::Request)),
            ErrorClass::RangeError
        );
    }

    #[test]
    fn wrong_json_type() {
        let d = info(json!({"type": "double"}));
        assert_eq!(
            class_of(d.decode(&json!("hot"), Mode::Request)),
            ErrorClass::WrongType
        );
    }

    #[test]
    fn numeric_values() {
        let d = info(json!({"type": "double"}));
        assert_eq!(d.decode(&json!(5), Mode::Request), Ok(Value::Double(5.0)));

        let i = info(json!({"type": "int", "min": 0, "max": 10}));
        assert_eq!(
            class_of(i.decode(&json!(2.5), Mode::Request)),
            ErrorClass::WrongType
        );
        assert_eq!(i.decode(&json!(3.0), Mode::Request), Ok(Value::Int(3)));
        assert_eq!(i.decode(&json!(10), Mode::Request), Ok(Value::Int(10)));
        assert_eq!(i.decode(&json!(0), Mode::Request), Ok(Value::Int(0)));
        assert_eq!(
            class_of(i.decode(&json!(11), Mode::Request)),
            ErrorClass::RangeError
        );
        assert_eq!(
            class_of(i.decode(&json!(1e30), Mode::Report)),
            ErrorClass::RangeError
        );
        assert_eq!(
            class_of(i.decode(&json!("3"), Mode::Report)),
            ErrorClass::WrongType
        );
    }

    #[test]
    fn scaled_conversion() {
        let s = info(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
        assert_eq!(
            s.decode(&json!(1255), Mode::Request),
            Ok(Value::Scaled(1255))
        );
        let DataInfo::Scaled(si) = &s else { panic!() };
        assert!((si.to_physical(1255) - 125.5).abs() < 1e-9);
        assert_eq!(si.from_physical(12.34), Some(123));
        assert_eq!(si.from_physical(f64::INFINITY), None);
        assert_eq!(
            class_of(s.decode(&json!(2501), Mode::Request)),
            ErrorClass::RangeError
        );
    }

    #[test]
    fn enum_values() {
        let e = info(json!({"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}));
        assert_eq!(e.decode(&json!(300), Mode::Report), Ok(Value::Enum(300)));
        assert_eq!(
            class_of(e.decode(&json!(250), Mode::Report)),
            ErrorClass::RangeError
        );
        assert_eq!(
            class_of(e.decode(&json!(250), Mode::Request)),
            ErrorClass::RangeError
        );
        assert_eq!(
            class_of(e.decode(&json!("IDLE"), Mode::Request)),
            ErrorClass::WrongType
        );
    }

    #[test]
    fn bool_values() {
        let b = info(json!({"type": "bool"}));
        assert_eq!(b.decode(&json!(true), Mode::Request), Ok(Value::Bool(true)));
        assert_eq!(
            class_of(b.decode(&json!(1), Mode::Request)),
            ErrorClass::WrongType
        );
    }

    // --- 4.3 dynamic limits

    #[test]
    fn dynamic_limits() {
        let d = info(json!({"type": "double", "min": 0, "max": 300}));
        let limits = DynLimits {
            min: Some(10.0),
            max: Some(200.0),
        };

        let e = d.decode_request(&json!(250.0), Some(&limits)).unwrap_err();
        assert_eq!(e.class, ErrorClass::RangeError);
        assert!(e.message.contains("dynamic upper limit 200"), "{e}");

        assert_eq!(
            d.decode_request(&json!(150.0), Some(&limits)),
            Ok(Value::Double(150.0))
        );

        let upper_only = DynLimits {
            min: None,
            max: Some(200.0),
        };
        let e = d
            .decode_request(&json!(-5.0), Some(&upper_only))
            .unwrap_err();
        assert!(
            e.message.contains("lower limit 0") && !e.message.contains("dynamic"),
            "{e}"
        );

        assert_eq!(
            d.decode_with(&json!(250.0), Mode::Report, Some(&limits)),
            Ok(Value::Double(250.0))
        );

        let s = info(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
        let max_1000 = DynLimits {
            min: None,
            max: Some(1000.0),
        };
        assert_eq!(
            class_of(s.decode_request(&json!(1500), Some(&max_1000))),
            ErrorClass::RangeError
        );

        let i = info(json!({"type": "int", "min": 0, "max": 100}));
        let lim =
            DynLimits::from_limits(&Value::Tuple(vec![Value::Int(5), Value::Int(50)])).unwrap();
        assert_eq!(
            class_of(i.decode_request(&json!(4), Some(&lim))),
            ErrorClass::RangeError
        );
        assert_eq!(i.decode_request(&json!(5), Some(&lim)), Ok(Value::Int(5)));
    }

    // --- 4.4 string and blob

    #[test]
    fn string_values() {
        let utf8 = info(json!({"type": "string", "maxchars": 3, "isUTF8": true}));
        assert_eq!(
            utf8.decode(&json!("äöü"), Mode::Request),
            Ok(Value::String("äöü".into()))
        );
        assert_eq!(
            class_of(utf8.decode(&json!("äöüß"), Mode::Request)),
            ErrorClass::RangeError
        );

        let ascii = info(json!({"type": "string"}));
        assert_eq!(
            class_of(ascii.decode(&json!("ä"), Mode::Request)),
            ErrorClass::RangeError
        );
        assert!(ascii.decode(&json!("ä"), Mode::Report).is_ok());

        let min = info(json!({"type": "string", "minchars": 2}));
        assert_eq!(
            class_of(min.decode(&json!("a"), Mode::Request)),
            ErrorClass::RangeError
        );
    }

    #[test]
    fn blob_values() {
        let b = info(json!({"type": "blob", "maxbytes": 8}));
        assert_eq!(
            b.decode(&json!("U0VDb1A="), Mode::Request),
            Ok(Value::Blob(b"SECoP".to_vec()))
        );
        assert_eq!(
            b.decode(&json!("AA=="), Mode::Request),
            Ok(Value::Blob(vec![0]))
        );
        assert_eq!(
            class_of(b.decode(&json!("not base64!"), Mode::Request)),
            ErrorClass::WrongType
        );
        assert_eq!(
            class_of(b.decode(&json!("U0VDb1A"), Mode::Request)),
            ErrorClass::WrongType
        );
        let small = info(json!({"type": "blob", "maxbytes": 4}));
        assert_eq!(
            class_of(small.decode(&json!("U0VDb1A="), Mode::Request)),
            ErrorClass::RangeError
        );
        assert_eq!(
            b.encode(&Value::Blob(b"SECoP".to_vec())),
            Ok(json!("U0VDb1A="))
        );
    }

    // --- 4.5 array, tuple, struct

    #[test]
    fn nested_failure_path() {
        let a = info(
            json!({"type": "array", "maxlen": 10, "members": {"type": "int", "min": 0, "max": 9}}),
        );
        let e = a.decode(&json!([1, 2, 99]), Mode::Request).unwrap_err();
        assert_eq!(e.class, ErrorClass::RangeError);
        assert_eq!(e.path.segments(), [PathSegment::Index(2)]);
    }

    #[test]
    fn array_and_tuple_values() {
        let a = info(json!({"type": "array", "maxlen": 3, "members": {"type": "bool"}}));
        assert_eq!(
            class_of(a.decode(&json!([true, true, true, true]), Mode::Request)),
            ErrorClass::RangeError
        );
        assert!(
            a.decode(&json!([true, true, true, true]), Mode::Report)
                .is_ok()
        );

        let t = info(
            json!({"type": "tuple", "members": [{"type": "int", "min": 0, "max": 999}, {"type": "string"}]}),
        );
        assert_eq!(
            class_of(t.decode(&json!([300]), Mode::Request)),
            ErrorClass::WrongType
        );
        assert_eq!(
            t.decode(&json!([300, "accelerating"]), Mode::Report),
            Ok(Value::Tuple(vec![
                Value::Int(300),
                Value::String("accelerating".into())
            ]))
        );
    }

    #[test]
    fn struct_values() {
        let s = info(json!({"type": "struct",
            "members": {"x": {"type": "double"}, "y": {"type": "double"}, "t": {"type": "double"}},
            "optional": ["t"]}));
        let Value::Struct(v) = s
            .decode(&json!({"x": 1.0, "y": 2.0}), Mode::Request)
            .unwrap()
        else {
            panic!()
        };
        assert!(!v.contains_key("t"));

        let e = s
            .decode(&json!({"x": 1.0, "y": 2.0}), Mode::Report)
            .unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert!(e.message.contains("`t`"), "{e}");

        let xy = info(
            json!({"type": "struct", "members": {"x": {"type": "double"}, "y": {"type": "double"}}}),
        );
        let e = xy
            .decode(&json!({"x": 1.0, "z": 0}), Mode::Request)
            .unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert!(e.message.contains("`z`"), "{e}");

        // members come out in datainfo order, regardless of input order
        let Value::Struct(v) = xy
            .decode(&json!({"y": 2.0, "x": 1.0}), Mode::Report)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(v.keys().collect::<Vec<_>>(), ["x", "y"]);
    }

    // --- 4.6 commands, NaN, shapes

    #[test]
    fn command_arguments() {
        let DataInfo::Command(c) = info(json!({"type": "command"})) else {
            panic!()
        };
        assert_eq!(c.decode_argument(None), Ok(None));
        assert_eq!(c.decode_argument(Some(&Json::Null)), Ok(None));
        let e = c.decode_argument(Some(&json!(5))).unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert_eq!(e.path.to_string(), "argument");
        assert_eq!(c.encode_result(None), Ok(Json::Null));

        let DataInfo::Command(c) = info(
            json!({"type": "command", "argument": {"type": "bool"}, "result": {"type": "int", "min": 0, "max": 9}}),
        ) else {
            panic!()
        };
        assert_eq!(
            c.decode_argument(Some(&json!(true))),
            Ok(Some(Value::Bool(true)))
        );
        assert_eq!(
            class_of(c.decode_argument(None).map(|_| Value::Bool(true))),
            ErrorClass::WrongType
        );
        assert_eq!(c.decode_result(&json!(42)), Ok(Some(Value::Int(42))));
        assert_eq!(c.encode_result(Some(&Value::Int(3))), Ok(json!(3)));
    }

    #[test]
    fn encode_refusals() {
        let d = info(json!({"type": "double"}));
        assert_eq!(
            d.encode(&Value::Double(f64::NAN)).unwrap_err().class,
            ErrorClass::WrongType
        );
        assert!(d.encode(&Value::Double(f64::INFINITY)).is_err());
        assert!(d.encode(&Value::Int(3)).is_err());
        let e = info(json!({"type": "enum", "members": {"a": 1}}));
        assert!(e.encode(&Value::Enum(2)).is_err());
        let s = info(json!({"type": "struct", "members": {"x": {"type": "double"}}}));
        assert!(s.encode(&Value::Struct(IndexMap::new())).is_err());
        let t = info(json!({"type": "tuple", "members": [{"type": "bool"}]}));
        assert!(t.encode(&Value::Tuple(vec![])).is_err());
    }

    #[test]
    fn struct_round_trip() {
        let s = info(json!({"type": "struct", "members": {
            "y": {"type": "double"},
            "x": {"type": "enum", "members": {"On": 1, "Off": 0}},
            "n": {"type": "array", "maxlen": 4, "members": {"type": "blob", "maxbytes": 3}}}}));
        let first = s
            .decode(
                &json!({"x": 1, "y": 0.5, "n": ["AA==", "AQI="]}),
                Mode::Report,
            )
            .unwrap();
        let second = s.decode(&s.encode(&first).unwrap(), Mode::Report).unwrap();
        assert_eq!(first, second);
    }
}
