// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP `datainfo` descriptions.
//!
//! A [`DataInfo`] describes the type of a parameter or the argument and result of a
//! command. It is parsed from and serialized to the JSON object found in a node's
//! `describe` reply. Every `DataInfo` value has passed validation: datainfos built in
//! code with [`DataInfoBuilder`] go through the same parser as JSON input.

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::identifier::{NameClash, NameScope};
use crate::path::{Path, PathSegment};

mod builder;
mod parse;
mod serialize;

pub use builder::DataInfoBuilder;
#[cfg(test)]
mod tests;

/// Default `relative_resolution` of `double` and `scaled`.
pub const DEFAULT_RELATIVE_RESOLUTION: f64 = 1.2e-7;
/// Default `fmtstr` of `double`.
pub const DEFAULT_DOUBLE_FMTSTR: &str = "%.6g";

/// A validated SECoP datainfo.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DataInfo {
    /// `double`: floating point number.
    Double(DoubleInfo),
    /// `scaled`: integer transported, physical value is integer × scale.
    Scaled(ScaledInfo),
    /// `int`: integer.
    Int(IntInfo),
    /// `bool`: boolean.
    Bool(BoolInfo),
    /// `enum`: named integer values.
    Enum(EnumInfo),
    /// `string`: human readable text.
    String(StringInfo),
    /// `blob`: binary data, transported as base64.
    Blob(BlobInfo),
    /// `array`: variable number of elements of one type.
    Array(ArrayInfo),
    /// `tuple`: fixed sequence of individually typed members.
    Tuple(TupleInfo),
    /// `struct`: named members.
    Struct(StructInfo),
    /// `matrix`: multi-dimensional numeric data, transported as lengths and base64 bytes.
    Matrix(MatrixInfo),
    /// `command`: argument and result of a command.
    Command(CommandInfo),
}

/// Properties of a `double` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct DoubleInfo {
    min: Option<f64>,
    max: Option<f64>,
    unit: Option<String>,
    absolute_resolution: Option<f64>,
    relative_resolution: Option<f64>,
    fmtstr: Option<String>,
    extra: Map<String, Json>,
}

/// Properties of a `scaled` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaledInfo {
    scale: f64,
    min: i64,
    max: i64,
    unit: Option<String>,
    absolute_resolution: Option<f64>,
    relative_resolution: Option<f64>,
    fmtstr: Option<String>,
    extra: Map<String, Json>,
}

/// Properties of an `int` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct IntInfo {
    min: i64,
    max: i64,
    unit: Option<String>,
    extra: Map<String, Json>,
}

/// Properties of a `bool` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct BoolInfo {
    extra: Map<String, Json>,
}

/// Properties of an `enum` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumInfo {
    members: IndexMap<String, i64>,
    extra: Map<String, Json>,
}

/// Properties of a `string` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct StringInfo {
    minchars: Option<u64>,
    maxchars: Option<u64>,
    is_utf8: Option<bool>,
    extra: Map<String, Json>,
}

/// Properties of a `blob` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct BlobInfo {
    minbytes: Option<u64>,
    maxbytes: u64,
    extra: Map<String, Json>,
}

/// Properties of an `array` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayInfo {
    members: Box<DataInfo>,
    minlen: Option<u64>,
    maxlen: u64,
    extra: Map<String, Json>,
}

/// Properties of a `tuple` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct TupleInfo {
    members: Vec<DataInfo>,
    extra: Map<String, Json>,
}

/// Properties of a `struct` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct StructInfo {
    members: IndexMap<String, DataInfo>,
    optional: Option<Vec<String>>,
    extra: Map<String, Json>,
}

/// Properties of a `matrix` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixInfo {
    elementtype: ElementType,
    names: Vec<String>,
    maxlen: Vec<u64>,
    extra: Map<String, Json>,
}

/// The kind of number in a matrix element: the middle letter of `elementtype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementKind {
    /// `i`: signed integer.
    Int,
    /// `u`: unsigned integer.
    UInt,
    /// `f`: IEEE 754 floating point number.
    Float,
}

/// The `elementtype` of a matrix, e.g. `<f4`: byte order, kind and size of one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementType {
    big_endian: bool,
    kind: ElementKind,
    bytes: u8,
}

impl ElementType {
    /// An element type, or `None` if `bytes` is not 1, 2, 4 or 8, or a float has 1 byte.
    pub fn new(big_endian: bool, kind: ElementKind, bytes: u8) -> Option<Self> {
        let valid = matches!(bytes, 1 | 2 | 4 | 8) && !(kind == ElementKind::Float && bytes == 1);
        valid.then_some(Self {
            big_endian,
            kind,
            bytes,
        })
    }

    /// Whether the elements are big-endian (`>`); little-endian (`<`) otherwise.
    pub fn big_endian(self) -> bool {
        self.big_endian
    }

    /// The kind of number.
    pub fn kind(self) -> ElementKind {
        self.kind
    }

    /// Size of one element in bytes: 1, 2, 4 or 8.
    pub fn size(self) -> usize {
        usize::from(self.bytes)
    }
}

impl fmt::Display for ElementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let order = if self.big_endian { '>' } else { '<' };
        let kind = match self.kind {
            ElementKind::Int => 'i',
            ElementKind::UInt => 'u',
            ElementKind::Float => 'f',
        };
        write!(f, "{order}{kind}{}", self.bytes)
    }
}

/// A string is not a valid matrix `elementtype`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid matrix element type `{0}`")]
pub struct InvalidElementType(pub String);

impl FromStr for ElementType {
    type Err = InvalidElementType;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let invalid = || InvalidElementType(s.to_owned());
        let &[order, kind, bytes] = s.as_bytes() else {
            return Err(invalid());
        };
        let big_endian = match order {
            b'<' => false,
            b'>' => true,
            _ => return Err(invalid()),
        };
        let kind = match kind {
            b'i' => ElementKind::Int,
            b'u' => ElementKind::UInt,
            b'f' => ElementKind::Float,
            _ => return Err(invalid()),
        };
        if !bytes.is_ascii_digit() {
            return Err(invalid());
        }
        Self::new(big_endian, kind, bytes - b'0').ok_or_else(invalid)
    }
}

/// Properties of a `command` datainfo.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandInfo {
    argument: Option<Box<DataInfo>>,
    result: Option<Box<DataInfo>>,
    extra: Map<String, Json>,
}

/// A datainfo failed validation.
#[derive(Debug, Clone, PartialEq, Error)]
pub struct DataInfoError {
    /// Where in a nested datainfo the problem is; empty for the top level.
    pub path: Path,
    /// What is wrong.
    pub kind: DataInfoErrorKind,
}

impl fmt::Display for DataInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_root() {
            write!(f, "{}", self.kind)
        } else {
            write!(f, "{}: {}", self.path, self.kind)
        }
    }
}

impl DataInfoError {
    fn new(kind: DataInfoErrorKind) -> Self {
        Self {
            path: Path::root(),
            kind,
        }
    }

    fn within(mut self, segment: PathSegment) -> Self {
        self.path = self.path.within(segment);
        self
    }
}

/// The ways a datainfo can be invalid.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum DataInfoErrorKind {
    /// The text is not valid JSON.
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    /// The datainfo is not a JSON object.
    #[error("datainfo must be a JSON object")]
    NotAnObject,
    /// The `type` property is missing or not a string.
    #[error("datainfo has no `type` string")]
    MissingType,
    /// The `type` is not a SECoP data type.
    #[error("unknown datainfo type `{0}`")]
    UnknownType(String),
    /// A property defined by SECoP that must does not support yet, such as the
    /// `compression` of a matrix.
    #[error("property `{0}` is not supported")]
    UnsupportedProperty(&'static str),
    /// A mandatory property of the type is missing.
    #[error("missing mandatory property `{0}`")]
    MissingProperty(&'static str),
    /// A known property has the wrong JSON type or an impossible value.
    #[error("property `{property}` must be {expected}")]
    WrongPropertyType {
        /// The property name.
        property: &'static str,
        /// What the property must be, e.g. "an integer".
        expected: &'static str,
    },
    /// A lower bound is greater than its upper bound.
    #[error("`{lower}` must not be greater than `{upper}`")]
    InvertedLimits {
        /// Name of the lower bound property.
        lower: &'static str,
        /// Name of the upper bound property.
        upper: &'static str,
    },
    /// The `scale` of a scaled datainfo is not a finite number greater than zero.
    #[error("`scale` must be a finite number greater than 0")]
    InvalidScale,
    /// The `fmtstr` does not follow the SECoP syntax.
    #[error("invalid `fmtstr` `{0}`, expected `%.<digits>` followed by `e`, `f` or `g`")]
    InvalidFmtstr(String),
    /// An enum or struct member has an empty name.
    #[error("member names must not be empty")]
    EmptyMemberName,
    /// Two enum or struct members differ only in case.
    #[error(transparent)]
    MemberNameClash(NameClash),
    /// Two enum members have the same value.
    #[error("enum members `{first}` and `{second}` have the same value {value}")]
    DuplicateEnumValue {
        /// The shared value.
        value: i64,
        /// The first member with that value.
        first: String,
        /// The second member with that value.
        second: String,
    },
    /// A struct's `optional` list names something that is not a member.
    #[error("`optional` lists `{0}`, which is not a member")]
    UnknownOptionalMember(String),
    /// A `command` datainfo appears inside another datainfo.
    #[error("a `command` datainfo cannot be nested")]
    NestedCommand,
}

type Result<T> = std::result::Result<T, DataInfoError>;

fn err<T>(kind: DataInfoErrorKind) -> Result<T> {
    Err(DataInfoError::new(kind))
}

// ---------------------------------------------------------------------------------------
// parsing helpers

/// Typed access to the properties of one datainfo object.
struct Props<'a> {
    obj: &'a Map<String, Json>,
    known: &'static [&'static str],
}

impl<'a> Props<'a> {
    fn get(&self, key: &'static str) -> Option<&'a Json> {
        debug_assert!(
            self.known.contains(&key),
            "`{key}` missing from known properties"
        );
        self.obj.get(key)
    }

    fn required<T>(
        &self,
        key: &'static str,
        conv: fn(&Self, &'static str) -> Result<Option<T>>,
    ) -> Result<T> {
        conv(self, key)?.map_or_else(|| err(DataInfoErrorKind::MissingProperty(key)), Ok)
    }

    fn f64(&self, key: &'static str) -> Result<Option<f64>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| wrong(key, "a number")),
        }
    }

    fn i64(&self, key: &'static str) -> Result<Option<i64>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => json_to_i64(v)
                .map(Some)
                .ok_or_else(|| wrong(key, "an integer")),
        }
    }

    fn u64(&self, key: &'static str) -> Result<Option<u64>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => json_to_u64(v)
                .map(Some)
                .ok_or_else(|| wrong(key, "a non-negative integer")),
        }
    }

    fn string(&self, key: &'static str) -> Result<Option<String>> {
        match self.get(key) {
            None => Ok(None),
            Some(Json::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(wrong(key, "a string")),
        }
    }

    fn bool(&self, key: &'static str) -> Result<Option<bool>> {
        match self.get(key) {
            None => Ok(None),
            Some(Json::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(wrong(key, "a boolean")),
        }
    }

    fn fmtstr(&self) -> Result<Option<String>> {
        let fmtstr = self.string("fmtstr")?;
        if let Some(f) = &fmtstr
            && !is_valid_fmtstr(f)
        {
            return err(DataInfoErrorKind::InvalidFmtstr(f.clone()));
        }
        Ok(fmtstr)
    }

    /// All properties that are neither `type` nor known for this datainfo type.
    fn extra(&self) -> Map<String, Json> {
        self.obj
            .iter()
            .filter(|(k, _)| k.as_str() != "type" && !self.known.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

fn wrong(property: &'static str, expected: &'static str) -> DataInfoError {
    DataInfoError::new(DataInfoErrorKind::WrongPropertyType { property, expected })
}

/// Integer from a JSON number, accepting floats with an integral value (e.g. `3.0`).
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub(crate) fn json_to_i64(v: &Json) -> Option<i64> {
    if let Some(i) = v.as_i64() {
        return Some(i);
    }
    let f = v.as_f64()?;
    // i64::MIN is exactly representable, i64::MAX + 1 is the first value out of range
    (f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64).then_some(f as i64)
}

/// Non-negative integer from a JSON number, accepting floats with an integral value.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn json_to_u64(v: &Json) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        return Some(u);
    }
    let f = v.as_f64()?;
    (f.fract() == 0.0 && f >= 0.0 && f < u64::MAX as f64).then_some(f as u64)
}

/// `%.` then an optional digit 1-9, one digit, and `e`, `f` or `g`.
fn is_valid_fmtstr(s: &str) -> bool {
    let b = s.as_bytes();
    let [b'%', b'.', digits @ .., b'e' | b'f' | b'g'] = b else {
        return false;
    };
    match digits {
        [d] => d.is_ascii_digit(),
        [d1, d2] => (b'1'..=b'9').contains(d1) && d2.is_ascii_digit(),
        _ => false,
    }
}

fn check_bounds<T: PartialOrd>(
    lower: Option<T>,
    upper: Option<T>,
    lower_name: &'static str,
    upper_name: &'static str,
) -> Result<()> {
    if let (Some(lo), Some(hi)) = (lower, upper)
        && lo > hi
    {
        return err(DataInfoErrorKind::InvertedLimits {
            lower: lower_name,
            upper: upper_name,
        });
    }
    Ok(())
}

/// Parses a nested datainfo, which may not be a command.
fn parse_nested(json: &Json, segment: PathSegment) -> Result<DataInfo> {
    let info = DataInfo::from_json(json).map_err(|e| e.within(segment.clone()))?;
    if matches!(info, DataInfo::Command(_)) {
        return Err(DataInfoError::new(DataInfoErrorKind::NestedCommand).within(segment));
    }
    Ok(info)
}

/// Checks a member name for emptiness and case-insensitive uniqueness.
fn add_member_name(scope: &mut NameScope, name: &str) -> Result<()> {
    if name.is_empty() {
        return err(DataInfoErrorKind::EmptyMemberName);
    }
    scope
        .insert(name)
        .map_err(|clash| DataInfoError::new(DataInfoErrorKind::MemberNameClash(clash)))
}

// ---------------------------------------------------------------------------------------
// accessors

impl DoubleInfo {
    /// Lower limit, if any.
    pub fn min(&self) -> Option<f64> {
        self.min
    }
    /// Upper limit, if any.
    pub fn max(&self) -> Option<f64> {
        self.max
    }
    /// Unit, if given.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
    /// Effective absolute resolution (default 0).
    pub fn absolute_resolution(&self) -> f64 {
        self.absolute_resolution.unwrap_or(0.0)
    }
    /// Effective relative resolution (default 1.2e-7).
    pub fn relative_resolution(&self) -> f64 {
        self.relative_resolution
            .unwrap_or(DEFAULT_RELATIVE_RESOLUTION)
    }
    /// Effective format string (default `%.6g`).
    pub fn fmtstr(&self) -> &str {
        self.fmtstr.as_deref().unwrap_or(DEFAULT_DOUBLE_FMTSTR)
    }
}

impl ScaledInfo {
    /// Scale factor between transported integer and physical value.
    pub fn scale(&self) -> f64 {
        self.scale
    }
    /// Lower limit of the transported integer.
    pub fn min(&self) -> i64 {
        self.min
    }
    /// Upper limit of the transported integer.
    pub fn max(&self) -> i64 {
        self.max
    }
    /// Unit of the physical value, if given.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
    /// Effective absolute resolution (default: the scale).
    pub fn absolute_resolution(&self) -> f64 {
        self.absolute_resolution.unwrap_or(self.scale)
    }
    /// Effective relative resolution (default 1.2e-7).
    pub fn relative_resolution(&self) -> f64 {
        self.relative_resolution
            .unwrap_or(DEFAULT_RELATIVE_RESOLUTION)
    }
    /// Effective format string; default `%.<n>f` with `n = max(0, -floor(log10(scale)))`.
    ///
    /// The SECoP prose defines this default; `schema/datatypes.yaml` of the
    /// specification says `%.6g` instead. MUST follows the prose.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn fmtstr(&self) -> String {
        if let Some(f) = &self.fmtstr {
            return f.clone();
        }
        // small epsilon so that e.g. log10(0.001) = -2.9999999999999996 still gives 3
        let digits = (-(self.scale.log10() + 1e-9).floor()).clamp(0.0, 99.0) as u8;
        format!("%.{digits}f")
    }
    /// Physical value of a transported integer: `raw × scale`.
    #[allow(clippy::cast_precision_loss)]
    pub fn to_physical(&self, raw: i64) -> f64 {
        raw as f64 * self.scale
    }
    /// Transported integer closest to a physical value, or `None` if out of `i64` range.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn from_physical(&self, physical: f64) -> Option<i64> {
        let raw = (physical / self.scale).round();
        (raw.is_finite() && raw >= i64::MIN as f64 && raw < i64::MAX as f64).then_some(raw as i64)
    }
}

impl IntInfo {
    /// Lower limit.
    pub fn min(&self) -> i64 {
        self.min
    }
    /// Upper limit.
    pub fn max(&self) -> i64 {
        self.max
    }
    /// Unit, if given.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
}

impl EnumInfo {
    /// Members as name → value, in description order.
    pub fn members(&self) -> &IndexMap<String, i64> {
        &self.members
    }
    /// Value of the member `name` (case sensitive).
    pub fn value_of(&self, name: &str) -> Option<i64> {
        self.members.get(name).copied()
    }
    /// Name of the member with `value`.
    pub fn name_of(&self, value: i64) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, v)| **v == value)
            .map(|(k, _)| k.as_str())
    }
}

impl StringInfo {
    /// Effective minimum length in characters (default 0).
    pub fn minchars(&self) -> u64 {
        self.minchars.unwrap_or(0)
    }
    /// Maximum length in characters, if limited.
    pub fn maxchars(&self) -> Option<u64> {
        self.maxchars
    }
    /// Whether characters outside 7-bit ASCII are allowed (default false).
    pub fn is_utf8(&self) -> bool {
        self.is_utf8.unwrap_or(false)
    }
}

impl BlobInfo {
    /// Effective minimum length in bytes (default 0).
    pub fn minbytes(&self) -> u64 {
        self.minbytes.unwrap_or(0)
    }
    /// Maximum length in bytes.
    pub fn maxbytes(&self) -> u64 {
        self.maxbytes
    }
}

impl ArrayInfo {
    /// Datainfo of the elements.
    pub fn members(&self) -> &DataInfo {
        &self.members
    }
    /// Effective minimum number of elements (default 0).
    pub fn minlen(&self) -> u64 {
        self.minlen.unwrap_or(0)
    }
    /// Maximum number of elements.
    pub fn maxlen(&self) -> u64 {
        self.maxlen
    }
}

impl TupleInfo {
    /// Datainfos of the members, in order.
    pub fn members(&self) -> &[DataInfo] {
        &self.members
    }
}

impl StructInfo {
    /// Members as name → datainfo, in description order.
    pub fn members(&self) -> &IndexMap<String, DataInfo> {
        &self.members
    }
    /// Whether `name` is listed in `optional`.
    pub fn is_optional(&self, name: &str) -> bool {
        self.optional
            .as_ref()
            .is_some_and(|o| o.iter().any(|n| n == name))
    }
    /// The `optional` list, empty if not given.
    pub fn optional(&self) -> &[String] {
        self.optional.as_deref().unwrap_or_default()
    }
}

impl MatrixInfo {
    /// Byte order, kind and size of the elements.
    pub fn elementtype(&self) -> ElementType {
        self.elementtype
    }
    /// Names of the dimensions, first (fastest varying) dimension first.
    pub fn names(&self) -> &[String] {
        &self.names
    }
    /// Maximum length of each dimension, in the order of [`names`](Self::names).
    pub fn maxlen(&self) -> &[u64] {
        &self.maxlen
    }
    /// Number of dimensions.
    pub fn dims(&self) -> usize {
        self.names.len()
    }
}

impl CommandInfo {
    /// Datainfo of the argument, `None` if the command takes no argument.
    pub fn argument(&self) -> Option<&DataInfo> {
        self.argument.as_deref()
    }
    /// Datainfo of the result, `None` if the command returns nothing.
    pub fn result(&self) -> Option<&DataInfo> {
        self.result.as_deref()
    }
}
