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
    ArrayInfo, BlobInfo, CommandInfo, DataInfo, DoubleInfo, ElementKind, EnumInfo, IntInfo,
    MatrixInfo, ScaledInfo, StringInfo, StructInfo, TupleInfo, json_to_i64,
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
    /// Value of a `matrix`: the length of each dimension and the raw element bytes, in
    /// transport order (first dimension varying fastest) and the byte order of the
    /// `elementtype`. See [`MatrixInfo`] for typed access to the elements.
    Matrix {
        /// Length of each dimension, in the order of [`MatrixInfo::names`].
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
            Self::Matrix(i) => i.decode(json, mode),
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

const MATRIX_MEMBERS: [&str; 2] = ["len", "blob"];

fn negative_len(index: usize, len: &dyn fmt::Display) -> ValueError {
    ValueError::wrong_type(format!("expected a non-negative integer, got {len}"))
        .within(PathSegment::Index(index))
        .within(PathSegment::Member("len".into()))
}

impl MatrixInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Object(items) = json else {
            return Err(expected("an object with `len` and `blob`", json));
        };
        if let Some(unknown) = items.keys().find(|k| !MATRIX_MEMBERS.contains(&k.as_str())) {
            return Err(ValueError::wrong_type(format!(
                "unknown matrix member `{unknown}`"
            )));
        }
        let member = |name: &str| {
            items
                .get(name)
                .ok_or_else(|| ValueError::wrong_type(format!("missing matrix member `{name}`")))
        };
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        let in_blob = |e: ValueError| e.within(PathSegment::Member("blob".into()));

        let json_len = member("len")?;
        let Json::Array(json_len) = json_len else {
            return Err(in_len(expected("an array of lengths", json_len)));
        };
        let len = json_len
            .iter()
            .enumerate()
            .map(|(i, l)| l.as_u64().ok_or_else(|| negative_len(i, l)))
            .collect::<Result<Vec<u64>>>()?;
        let len = self.lengths(&len, mode)?;

        let json_blob = member("blob")?;
        let Json::String(blob) = json_blob else {
            return Err(in_blob(expected("a base64 string", json_blob)));
        };
        let data = BASE64
            .decode(blob)
            .map_err(|e| in_blob(ValueError::wrong_type(format!("invalid base64: {e}"))))?;
        self.check_bytes(&len, data.len())?;
        Ok(Value::Matrix { len, data })
    }

    /// Lengths as `u32`, checked as decoding checks the `len` member: one per dimension
    /// (`WrongType`), in [`Mode::Request`] none above its `maxlen` (`RangeError`), and
    /// each within `u32` (`WrongType`).
    pub fn lengths(&self, len: &[u64], mode: Mode) -> Result<Vec<u32>> {
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        self.check_dims(len.len()).map_err(in_len)?;
        if mode == Mode::Request {
            self.check_maxlen(len).map_err(in_len)?;
        }
        len.iter()
            .enumerate()
            .map(|(i, &l)| {
                u32::try_from(l).map_err(|_| {
                    in_len(
                        ValueError::wrong_type(format!("{l} is too long for a dimension"))
                            .within(PathSegment::Index(i)),
                    )
                })
            })
            .collect()
    }

    /// Lengths given as signed integers, as a C host holds them: a negative one is the
    /// same `WrongType` error as in JSON, the rest as [`lengths`](Self::lengths).
    pub fn signed_lengths(&self, len: &[i64], mode: Mode) -> Result<Vec<u32>> {
        let len = len
            .iter()
            .enumerate()
            .map(|(i, &l)| u64::try_from(l).map_err(|_| negative_len(i, &l)))
            .collect::<Result<Vec<u64>>>()?;
        self.lengths(&len, mode)
    }

    /// A matrix value from lengths and element bytes, checked as by
    /// [`check`](Self::check).
    pub fn value(&self, len: Vec<u32>, data: Vec<u8>, mode: Mode) -> Result<Value> {
        self.check(&len, data.len(), mode)?;
        Ok(Value::Matrix { len, data })
    }

    /// Checks lengths and the number of element bytes against the datainfo, as decoding
    /// does: one length per dimension and exactly `product(len) × element size` bytes
    /// (`WrongType`), and in [`Mode::Request`] no length above its `maxlen`
    /// (`RangeError`). Error paths point at `len` or `blob`, as for decoded JSON.
    pub fn check(&self, len: &[u32], bytes: usize, mode: Mode) -> Result<()> {
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        self.check_dims(len.len()).map_err(in_len)?;
        if mode == Mode::Request {
            let wide: Vec<u64> = len.iter().map(|&l| u64::from(l)).collect();
            self.check_maxlen(&wide).map_err(in_len)?;
        }
        self.check_bytes(len, bytes)
    }

    fn check_dims(&self, dims: usize) -> Result<()> {
        if dims == self.dims() {
            Ok(())
        } else {
            Err(ValueError::wrong_type(format!(
                "expected {} dimensions, got {dims}",
                self.dims()
            )))
        }
    }

    fn check_maxlen(&self, len: &[u64]) -> Result<()> {
        for (i, (&l, &max)) in len.iter().zip(self.maxlen()).enumerate() {
            if l > max {
                return Err(ValueError::range(format!(
                    "{l} elements along `{}`, at most {max} allowed",
                    self.names()[i]
                ))
                .within(PathSegment::Index(i)));
            }
        }
        Ok(())
    }

    fn check_bytes(&self, len: &[u32], bytes: usize) -> Result<()> {
        let Some(required) = self.byte_count(len) else {
            return Err(ValueError::wrong_type(format!(
                "the lengths {len:?} give more bytes than this platform can hold"
            ))
            .within(PathSegment::Member("len".into())));
        };
        if bytes == required {
            Ok(())
        } else {
            Err(ValueError::wrong_type(format!(
                "the lengths {len:?} require {required} bytes, got {bytes}"
            ))
            .within(PathSegment::Member("blob".into())))
        }
    }

    // --- typed access

    /// Number of elements of a matrix with lengths `len`, `None` if it overflows `usize`.
    pub fn element_count(len: &[u32]) -> Option<usize> {
        len.iter().try_fold(1usize, |count, &l| {
            count.checked_mul(usize::try_from(l).ok()?)
        })
    }

    /// Number of bytes of a matrix with lengths `len`, `None` if it overflows `usize`.
    pub fn byte_count(&self, len: &[u32]) -> Option<usize> {
        Self::element_count(len)?.checked_mul(self.elementtype().size())
    }

    /// Element `index` of `data` (in transport order) as `f64`. `None` if `index` is out
    /// of range or the elements are half floats (`f2`), which pass through as bytes only.
    /// 64-bit integers beyond ±2⁵³ lose precision.
    #[allow(clippy::cast_precision_loss)]
    pub fn element_f64(&self, data: &[u8], index: usize) -> Option<f64> {
        let t = self.elementtype();
        let size = t.size();
        let start = index.checked_mul(size)?;
        let bytes = data.get(start..start.checked_add(size)?)?;
        let mut buf = [0u8; 8];
        // place the element's bytes so that from_le/from_be of an 8-byte array works
        if t.big_endian() {
            buf[8 - size..].copy_from_slice(bytes);
        } else {
            buf[..size].copy_from_slice(bytes);
        }
        let raw = if t.big_endian() {
            u64::from_be_bytes(buf)
        } else {
            u64::from_le_bytes(buf)
        };
        let bits = u32::try_from(size * 8).ok()?;
        Some(match t.kind() {
            ElementKind::UInt => raw as f64,
            ElementKind::Int => {
                // sign-extend from the element width
                let shift = 64 - bits;
                (((raw << shift).cast_signed()) >> shift) as f64
            }
            ElementKind::Float => match size {
                4 => f64::from(f32::from_bits(u32::try_from(raw).ok()?)),
                8 => f64::from_bits(raw),
                _ => return None,
            },
        })
    }

    /// All elements of `data` as `f64`s, in transport order; `None` for half floats or a
    /// byte count that is not a multiple of the element size. See
    /// [`element_f64`](Self::element_f64).
    pub fn to_f64s(&self, data: &[u8]) -> Option<Vec<f64>> {
        let size = self.elementtype().size();
        if !data.len().is_multiple_of(size) {
            return None;
        }
        (0..data.len() / size)
            .map(|i| self.element_f64(data, i))
            .collect()
    }

    /// A matrix value with lengths `len` from elements given as `f64`s in transport order.
    ///
    /// Integer elements must be integral and within the range of the element type, float
    /// elements of `f4` must be within the `f32` range (NaN and infinities are kept);
    /// otherwise a `RangeError` points at the element's index. Half floats (`f2`) are not
    /// supported (`WrongType`); build their bytes directly. A count of values other than
    /// `product(len)`, or a wrong number of lengths, is a `WrongType` error.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn from_f64s(&self, len: &[u32], values: &[f64]) -> Result<Value> {
        let t = self.elementtype();
        let size = t.size();
        self.check_dims(len.len())
            .map_err(|e| e.within(PathSegment::Member("len".into())))?;
        if Self::element_count(len) != Some(values.len()) {
            return Err(ValueError::wrong_type(format!(
                "the lengths {len:?} do not fit {} values",
                values.len()
            )));
        }
        if t.kind() == ElementKind::Float && size == 2 {
            return Err(ValueError::wrong_type(
                "half float (`f2`) elements cannot be built from f64 values",
            ));
        }
        let bits = size * 8;
        let mut data = Vec::with_capacity(values.len() * size);
        for (i, &v) in values.iter().enumerate() {
            let out_of_range = || {
                ValueError::range(format!("{v} does not fit `{t}`")).within(PathSegment::Index(i))
            };
            let raw: u64 = match t.kind() {
                ElementKind::Float if size == 4 => {
                    let f = v as f32;
                    if v.is_finite() && f.is_infinite() {
                        return Err(out_of_range());
                    }
                    u64::from(f.to_bits())
                }
                ElementKind::Float => v.to_bits(),
                ElementKind::UInt => {
                    // 2^bits as f64 is exact; the upper limit is exclusive
                    let limit = 2f64.powi(i32::try_from(bits).unwrap_or(64));
                    if v.fract() != 0.0 || !(0.0..limit).contains(&v) {
                        return Err(out_of_range());
                    }
                    v as u64
                }
                ElementKind::Int => {
                    let limit = 2f64.powi(i32::try_from(bits - 1).unwrap_or(63));
                    if v.fract() != 0.0 || !(-limit..limit).contains(&v) {
                        return Err(out_of_range());
                    }
                    (v as i64).cast_unsigned()
                }
            };
            if t.big_endian() {
                data.extend_from_slice(&raw.to_be_bytes()[8 - size..]);
            } else {
                data.extend_from_slice(&raw.to_le_bytes()[..size]);
            }
        }
        Ok(Value::Matrix {
            len: len.to_vec(),
            data,
        })
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
            (Self::Matrix(info), Value::Matrix { len, data }) => {
                info.check(len, data.len(), Mode::Report)?;
                let mut out = Map::new();
                out.insert("len".into(), len.clone().into());
                out.insert("blob".into(), BASE64.encode(data).into());
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
        Value::Matrix { .. } => "matrix",
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

    // --- matrix

    const SPEC_BLOB: &str = "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA";

    fn image() -> DataInfo {
        info(
            json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}),
        )
    }

    fn matrix_info(info: &DataInfo) -> &MatrixInfo {
        let DataInfo::Matrix(m) = info else {
            panic!("not a matrix")
        };
        m
    }

    #[test]
    fn matrix_spec_example() {
        let m = image();
        let value = m
            .decode(&json!({"len": [2, 3], "blob": SPEC_BLOB}), Mode::Request)
            .unwrap();
        let (len, data) = value.as_matrix().unwrap();
        assert_eq!(len, [2, 3]);
        assert_eq!(data.len(), 24);
        assert_eq!(
            matrix_info(&m).to_f64s(data).unwrap(),
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn matrix_blob_size_does_not_fit() {
        let e = image()
            .decode(&json!({"len": [2, 3], "blob": "AACAPw=="}), Mode::Report)
            .unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert_eq!(e.path.to_string(), "blob");
        assert!(e.to_string().contains("blob"), "{e}");
    }

    #[test]
    fn matrix_wrong_number_of_dimensions() {
        let e = image()
            .decode(&json!({"len": [6], "blob": SPEC_BLOB}), Mode::Report)
            .unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert_eq!(e.path.to_string(), "len");
    }

    #[test]
    fn matrix_dimension_beyond_maxlen() {
        let blob = BASE64.encode(vec![0u8; 404]);
        let json = json!({"len": [101, 1], "blob": blob});
        let e = image().decode(&json, Mode::Request).unwrap_err();
        assert_eq!(e.class, ErrorClass::RangeError);
        assert_eq!(
            e.path.segments(),
            [PathSegment::Member("len".into()), PathSegment::Index(0)]
        );
        assert_eq!(e.path.to_string(), "len[0]");
        // reports are only checked for their shape
        assert!(image().decode(&json, Mode::Report).is_ok());
    }

    #[test]
    fn matrix_empty() {
        let value = image()
            .decode(&json!({"len": [0, 0], "blob": ""}), Mode::Request)
            .unwrap();
        assert_eq!(value.as_matrix(), Some((&[0, 0][..], &[][..])));
        assert_eq!(
            image().encode(&value),
            Ok(json!({"len": [0, 0], "blob": ""}))
        );
        // one empty dimension makes the whole matrix empty
        assert!(
            image()
                .decode(&json!({"len": [5, 0], "blob": ""}), Mode::Request)
                .is_ok()
        );
    }

    #[test]
    fn matrix_unknown_member() {
        let e = image()
            .decode(
                &json!({"len": [1, 1], "blob": "AACAPw==", "t": 0}),
                Mode::Report,
            )
            .unwrap_err();
        assert_eq!(e.class, ErrorClass::WrongType);
        assert!(e.message.contains("`t`"), "{e}");
    }

    #[test]
    fn matrix_shapes() {
        let m = image();
        for json in [
            json!([[1.0]]),
            json!({"len": [1, 1]}),
            json!({"blob": "AACAPw=="}),
            json!({"len": "1", "blob": "AACAPw=="}),
            json!({"len": [1, -1], "blob": "AACAPw=="}),
            json!({"len": [1, 1.5], "blob": "AACAPw=="}),
            json!({"len": [1, 1], "blob": [0, 0, 128, 63]}),
            json!({"len": [1, 1], "blob": "AACAPw"}),
            json!({"len": [1, 1], "blob": "AACA\nPw=="}),
            json!({"len": [4_294_967_296_u64, 1], "blob": ""}),
            json!({"len": [4_294_967_295_u32, 4_294_967_295_u32], "blob": ""}),
        ] {
            assert_eq!(
                class_of(m.decode(&json, Mode::Report)),
                ErrorClass::WrongType,
                "{json}"
            );
        }
        // the product of the lengths overflows on every platform
        let wide = info(json!({"type": "matrix", "elementtype": "<f8",
            "names": ["a", "b", "c"], "maxlen": [4_294_967_295_u32, 4_294_967_295_u32, 4_294_967_295_u32]}));
        let e = wide
            .decode(
                &json!({"len": [4_294_967_295_u32, 4_294_967_295_u32, 4_294_967_295_u32], "blob": ""}),
                Mode::Request,
            )
            .unwrap_err();
        assert_eq!(
            (e.class, e.path.to_string()),
            (ErrorClass::WrongType, "len".into())
        );
    }

    #[test]
    fn matrix_round_trip() {
        let m = image();
        let json = json!({"len": [2, 3], "blob": SPEC_BLOB});
        let first = m.decode(&json, Mode::Report).unwrap();
        let encoded = m.encode(&first).unwrap();
        assert_eq!(encoded, json);
        assert_eq!(encoded["blob"], SPEC_BLOB);
        assert_eq!(m.decode(&encoded, Mode::Report).unwrap(), first);
    }

    #[test]
    fn matrix_encode_checks_shape() {
        let m = image();
        let short = Value::Matrix {
            len: vec![2, 3],
            data: vec![0; 4],
        };
        let e = m.encode(&short).unwrap_err();
        assert_eq!(
            (e.class, e.path.to_string()),
            (ErrorClass::WrongType, "blob".into())
        );
        let flat = Value::Matrix {
            len: vec![6],
            data: vec![0; 24],
        };
        assert_eq!(m.encode(&flat).unwrap_err().path.to_string(), "len");
        assert!(m.encode(&Value::Blob(vec![])).is_err());
        // limits are not checked when encoding
        let big = Value::Matrix {
            len: vec![101, 1],
            data: vec![0; 404],
        };
        assert!(m.encode(&big).is_ok());
        assert_eq!(
            matrix_info(&m)
                .check(&[101, 1], 404, Mode::Request)
                .unwrap_err()
                .class,
            ErrorClass::RangeError
        );
        assert!(matrix_info(&m).check(&[101, 1], 404, Mode::Report).is_ok());
    }

    // --- matrix helpers

    fn matrix_of(elementtype: &str) -> DataInfo {
        info(json!({"type": "matrix", "elementtype": elementtype, "names": ["x"], "maxlen": [16]}))
    }

    #[test]
    fn matrix_helpers_both_endiannesses() {
        let values = [1.0, -2.5, 3.25];
        for t in ["<f4", ">f4", "<f8", ">f8"] {
            let info = matrix_of(t);
            let m = matrix_info(&info);
            let value = m.from_f64s(&[3], &values).unwrap();
            let (_, data) = value.as_matrix().unwrap();
            assert_eq!(data.len(), 3 * m.elementtype().size(), "{t}");
            assert_eq!(m.to_f64s(data).unwrap(), values, "{t}");
            assert_eq!(m.element_f64(data, 1), Some(-2.5), "{t}");
            assert_eq!(m.element_f64(data, 3), None, "{t}");
        }
        let le = matrix_of("<u2");
        let be = matrix_of(">u2");
        let Value::Matrix { data: le_data, .. } =
            matrix_info(&le).from_f64s(&[1], &[258.0]).unwrap()
        else {
            panic!()
        };
        let Value::Matrix { data: be_data, .. } =
            matrix_info(&be).from_f64s(&[1], &[258.0]).unwrap()
        else {
            panic!()
        };
        assert_eq!((le_data, be_data), (vec![2, 1], vec![1, 2]));
        assert_eq!(
            matrix_info(&be).element_f64(&[0xff, 0xfe], 0),
            Some(65534.0)
        );
        assert_eq!(
            matrix_info(&matrix_of(">i2")).element_f64(&[0xff, 0xfe], 0),
            Some(-2.0)
        );
        assert_eq!(
            matrix_info(&matrix_of("<i2")).element_f64(&[0xfe, 0xff], 0),
            Some(-2.0)
        );
    }

    #[test]
    fn matrix_helpers_extremes() {
        let u8_info = matrix_of("<u1");
        let u8m = matrix_info(&u8_info);
        let v = u8m.from_f64s(&[2], &[0.0, 255.0]).unwrap();
        assert_eq!(v.as_matrix().unwrap().1, [0, 255]);
        assert_eq!(u8m.to_f64s(&[0, 255]).unwrap(), [0.0, 255.0]);
        for bad in [256.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
            let e = u8m.from_f64s(&[2], &[0.0, bad]).unwrap_err();
            assert_eq!(e.class, ErrorClass::RangeError, "{bad}");
            assert_eq!(e.path.segments(), [PathSegment::Index(1)]);
        }

        #[allow(clippy::cast_precision_loss)]
        let (min, max) = (i64::MIN as f64, i64::MAX as f64);
        for t in ["<i8", ">i8"] {
            let info = matrix_of(t);
            let m = matrix_info(&info);
            // i64::MAX as f64 is 2^63, one beyond the range
            assert!(m.from_f64s(&[1], &[max]).is_err(), "{t}");
            let v = m.from_f64s(&[2], &[min, -1.0]).unwrap();
            let (_, data) = v.as_matrix().unwrap();
            assert_eq!(m.to_f64s(data).unwrap(), [min, -1.0]);
            let mut extreme = i64::MIN.to_le_bytes();
            if t.starts_with('>') {
                extreme = i64::MIN.to_be_bytes();
            }
            assert_eq!(m.element_f64(&extreme, 0), Some(min));
        }
        let i1_info = matrix_of("<i1");
        let i1 = matrix_info(&i1_info);
        assert_eq!(i1.to_f64s(&[0x80, 0x7f]).unwrap(), [-128.0, 127.0]);
        assert!(i1.from_f64s(&[1], &[128.0]).is_err());
        assert!(i1.from_f64s(&[1], &[-128.0]).is_ok());

        let f4_info = matrix_of("<f4");
        let f4 = matrix_info(&f4_info);
        assert_eq!(
            f4.from_f64s(&[1], &[1e300]).unwrap_err().class,
            ErrorClass::RangeError
        );
        let v = f4.from_f64s(&[2], &[f64::NAN, f64::NEG_INFINITY]).unwrap();
        let back = f4.to_f64s(v.as_matrix().unwrap().1).unwrap();
        assert!(back[0].is_nan() && back[1] == f64::NEG_INFINITY);
    }

    #[test]
    fn matrix_helpers_refusals() {
        let f2_info = matrix_of("<f2");
        let f2 = matrix_info(&f2_info);
        assert_eq!(f2.element_f64(&[0, 0x3c], 0), None);
        assert_eq!(f2.to_f64s(&[0, 0x3c]), None);
        assert_eq!(
            f2.from_f64s(&[1], &[1.0]).unwrap_err().class,
            ErrorClass::WrongType
        );
        // f2 data still passes through as bytes
        let value = f2_info
            .decode(&json!({"len": [1], "blob": "ADw="}), Mode::Request)
            .unwrap();
        assert_eq!(value.as_matrix().unwrap().1, [0, 0x3c]);

        let f4_info = matrix_of("<f4");
        let f4 = matrix_info(&f4_info);
        assert_eq!(
            f4.from_f64s(&[3], &[1.0]).unwrap_err().class,
            ErrorClass::WrongType
        );
        assert_eq!(
            f4.from_f64s(&[1, 1], &[1.0]).unwrap_err().class,
            ErrorClass::WrongType
        );
        assert_eq!(f4.to_f64s(&[0, 0, 0]), None);
        assert_eq!(MatrixInfo::element_count(&[2, 3, 4]), Some(24));
        assert_eq!(MatrixInfo::element_count(&[]), Some(1));
        assert_eq!(f4.byte_count(&[2, 3]), Some(24));
    }
}
