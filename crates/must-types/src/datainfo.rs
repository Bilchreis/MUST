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
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json};
use thiserror::Error;

use crate::identifier::{NameClash, NameScope};
use crate::path::{Path, PathSegment};

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
    /// The `type` is a SECoP data type that must does not support yet.
    #[error("datainfo type `{0}` is not supported")]
    UnsupportedType(String),
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
// parsing per type

impl DataInfo {
    /// Parses and validates a datainfo from its JSON representation.
    pub fn from_json(json: &Json) -> Result<Self> {
        let Json::Object(obj) = json else {
            return err(DataInfoErrorKind::NotAnObject);
        };
        let Some(Json::String(ty)) = obj.get("type") else {
            return err(DataInfoErrorKind::MissingType);
        };
        match ty.as_str() {
            "double" => DoubleInfo::parse(obj).map(Self::Double),
            "scaled" => ScaledInfo::parse(obj).map(Self::Scaled),
            "int" => IntInfo::parse(obj).map(Self::Int),
            "bool" => Ok(Self::Bool(BoolInfo {
                extra: Props { obj, known: &[] }.extra(),
            })),
            "enum" => EnumInfo::parse(obj).map(Self::Enum),
            "string" => StringInfo::parse(obj).map(Self::String),
            "blob" => BlobInfo::parse(obj).map(Self::Blob),
            "array" => ArrayInfo::parse(obj).map(Self::Array),
            "tuple" => TupleInfo::parse(obj).map(Self::Tuple),
            "struct" => StructInfo::parse(obj).map(Self::Struct),
            "command" => CommandInfo::parse(obj).map(Self::Command),
            "matrix" => err(DataInfoErrorKind::UnsupportedType(ty.clone())),
            _ => err(DataInfoErrorKind::UnknownType(ty.clone())),
        }
    }
}

impl DoubleInfo {
    const KNOWN: &[&str] = &[
        "min",
        "max",
        "unit",
        "absolute_resolution",
        "relative_resolution",
        "fmtstr",
    ];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let info = Self {
            min: p.f64("min")?,
            max: p.f64("max")?,
            unit: p.string("unit")?,
            absolute_resolution: p.f64("absolute_resolution")?,
            relative_resolution: p.f64("relative_resolution")?,
            fmtstr: p.fmtstr()?,
            extra: p.extra(),
        };
        check_bounds(info.min, info.max, "min", "max")?;
        Ok(info)
    }
}

impl ScaledInfo {
    const KNOWN: &[&str] = &[
        "scale",
        "min",
        "max",
        "unit",
        "absolute_resolution",
        "relative_resolution",
        "fmtstr",
    ];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let info = Self {
            scale: p.required("scale", Props::f64)?,
            min: p.required("min", Props::i64)?,
            max: p.required("max", Props::i64)?,
            unit: p.string("unit")?,
            absolute_resolution: p.f64("absolute_resolution")?,
            relative_resolution: p.f64("relative_resolution")?,
            fmtstr: p.fmtstr()?,
            extra: p.extra(),
        };
        if !(info.scale.is_finite() && info.scale > 0.0) {
            return err(DataInfoErrorKind::InvalidScale);
        }
        check_bounds(Some(info.min), Some(info.max), "min", "max")?;
        Ok(info)
    }
}

impl IntInfo {
    const KNOWN: &[&str] = &["min", "max", "unit"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let info = Self {
            min: p.required("min", Props::i64)?,
            max: p.required("max", Props::i64)?,
            unit: p.string("unit")?,
            extra: p.extra(),
        };
        check_bounds(Some(info.min), Some(info.max), "min", "max")?;
        Ok(info)
    }
}

impl EnumInfo {
    const KNOWN: &[&str] = &["members"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let Some(json_members) = p.get("members") else {
            return err(DataInfoErrorKind::MissingProperty("members"));
        };
        let Json::Object(json_members) = json_members else {
            return Err(wrong("members", "an object of names to integers"));
        };
        let mut scope = NameScope::new();
        let mut members: IndexMap<String, i64> = IndexMap::new();
        for (name, value) in json_members {
            add_member_name(&mut scope, name)?;
            let value = json_to_i64(value)
                .ok_or_else(|| wrong("members", "an object of names to integers"))?;
            if let Some((first, _)) = members.iter().find(|(_, v)| **v == value) {
                return err(DataInfoErrorKind::DuplicateEnumValue {
                    value,
                    first: first.clone(),
                    second: name.clone(),
                });
            }
            members.insert(name.clone(), value);
        }
        Ok(Self {
            members,
            extra: p.extra(),
        })
    }
}

impl StringInfo {
    const KNOWN: &[&str] = &["minchars", "maxchars", "isUTF8"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let info = Self {
            minchars: p.u64("minchars")?,
            maxchars: p.u64("maxchars")?,
            is_utf8: p.bool("isUTF8")?,
            extra: p.extra(),
        };
        check_bounds(info.minchars, info.maxchars, "minchars", "maxchars")?;
        Ok(info)
    }
}

impl BlobInfo {
    const KNOWN: &[&str] = &["minbytes", "maxbytes"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let info = Self {
            minbytes: p.u64("minbytes")?,
            maxbytes: p.required("maxbytes", Props::u64)?,
            extra: p.extra(),
        };
        check_bounds(info.minbytes, Some(info.maxbytes), "minbytes", "maxbytes")?;
        Ok(info)
    }
}

impl ArrayInfo {
    const KNOWN: &[&str] = &["members", "minlen", "maxlen"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let Some(json_members) = p.get("members") else {
            return err(DataInfoErrorKind::MissingProperty("members"));
        };
        let members = Box::new(parse_nested(json_members, PathSegment::Element)?);
        let info = Self {
            members,
            minlen: p.u64("minlen")?,
            maxlen: p.required("maxlen", Props::u64)?,
            extra: p.extra(),
        };
        check_bounds(info.minlen, Some(info.maxlen), "minlen", "maxlen")?;
        Ok(info)
    }
}

impl TupleInfo {
    const KNOWN: &[&str] = &["members"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let Some(json_members) = p.get("members") else {
            return err(DataInfoErrorKind::MissingProperty("members"));
        };
        let Json::Array(json_members) = json_members else {
            return Err(wrong("members", "a list of datainfos"));
        };
        let members = json_members
            .iter()
            .enumerate()
            .map(|(i, m)| parse_nested(m, PathSegment::Index(i)))
            .collect::<Result<_>>()?;
        Ok(Self {
            members,
            extra: p.extra(),
        })
    }
}

impl StructInfo {
    const KNOWN: &[&str] = &["members", "optional"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let Some(json_members) = p.get("members") else {
            return err(DataInfoErrorKind::MissingProperty("members"));
        };
        let Json::Object(json_members) = json_members else {
            return Err(wrong("members", "an object of names to datainfos"));
        };
        let mut scope = NameScope::new();
        let mut members = IndexMap::new();
        for (name, member) in json_members {
            add_member_name(&mut scope, name)?;
            members.insert(
                name.clone(),
                parse_nested(member, PathSegment::Member(name.clone()))?,
            );
        }
        let optional = match p.get("optional") {
            None => None,
            Some(Json::Array(names)) => {
                let names = names
                    .iter()
                    .map(|n| n.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| wrong("optional", "a list of member names"))?;
                if let Some(unknown) = names.iter().find(|n| !members.contains_key(*n)) {
                    return err(DataInfoErrorKind::UnknownOptionalMember(unknown.clone()));
                }
                Some(names)
            }
            Some(_) => return Err(wrong("optional", "a list of member names")),
        };
        Ok(Self {
            members,
            optional,
            extra: p.extra(),
        })
    }
}

impl CommandInfo {
    const KNOWN: &[&str] = &["argument", "result"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        let nested = |key, segment| -> Result<Option<Box<DataInfo>>> {
            match p.get(key) {
                None | Some(Json::Null) => Ok(None),
                Some(json) => parse_nested(json, segment).map(|i| Some(Box::new(i))),
            }
        };
        Ok(Self {
            argument: nested("argument", PathSegment::Argument)?,
            result: nested("result", PathSegment::Result)?,
            extra: p.extra(),
        })
    }
}

// ---------------------------------------------------------------------------------------
// serialization

/// Builds a JSON object with `type` first, then the given properties in order.
struct ObjectWriter(Map<String, Json>);

impl ObjectWriter {
    fn new(ty: &str) -> Self {
        let mut map = Map::new();
        map.insert("type".into(), Json::from(ty));
        Self(map)
    }

    fn opt(mut self, key: &str, value: Option<impl Into<Json>>) -> Self {
        if let Some(v) = value {
            self.0.insert(key.into(), v.into());
        }
        self
    }

    fn set(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    fn finish(mut self, extra: &Map<String, Json>) -> Json {
        for (k, v) in extra {
            self.0.insert(k.clone(), v.clone());
        }
        Json::Object(self.0)
    }
}

impl DataInfo {
    /// Serializes the datainfo to JSON, with only the properties that were given.
    pub fn to_json(&self) -> Json {
        match self {
            Self::Double(i) => ObjectWriter::new("double")
                .opt("min", i.min)
                .opt("max", i.max)
                .opt("unit", i.unit.clone())
                .opt("absolute_resolution", i.absolute_resolution)
                .opt("relative_resolution", i.relative_resolution)
                .opt("fmtstr", i.fmtstr.clone())
                .finish(&i.extra),
            Self::Scaled(i) => ObjectWriter::new("scaled")
                .set("scale", i.scale)
                .set("min", i.min)
                .set("max", i.max)
                .opt("unit", i.unit.clone())
                .opt("absolute_resolution", i.absolute_resolution)
                .opt("relative_resolution", i.relative_resolution)
                .opt("fmtstr", i.fmtstr.clone())
                .finish(&i.extra),
            Self::Int(i) => ObjectWriter::new("int")
                .set("min", i.min)
                .set("max", i.max)
                .opt("unit", i.unit.clone())
                .finish(&i.extra),
            Self::Bool(i) => ObjectWriter::new("bool").finish(&i.extra),
            Self::Enum(i) => {
                let members: Map<String, Json> = i
                    .members
                    .iter()
                    .map(|(k, v)| (k.clone(), Json::from(*v)))
                    .collect();
                ObjectWriter::new("enum")
                    .set("members", members)
                    .finish(&i.extra)
            }
            Self::String(i) => ObjectWriter::new("string")
                .opt("minchars", i.minchars)
                .opt("maxchars", i.maxchars)
                .opt("isUTF8", i.is_utf8)
                .finish(&i.extra),
            Self::Blob(i) => ObjectWriter::new("blob")
                .opt("minbytes", i.minbytes)
                .set("maxbytes", i.maxbytes)
                .finish(&i.extra),
            Self::Array(i) => ObjectWriter::new("array")
                .set("members", i.members.to_json())
                .opt("minlen", i.minlen)
                .set("maxlen", i.maxlen)
                .finish(&i.extra),
            Self::Tuple(i) => {
                let members: Vec<Json> = i.members.iter().map(Self::to_json).collect();
                ObjectWriter::new("tuple")
                    .set("members", members)
                    .finish(&i.extra)
            }
            Self::Struct(i) => {
                let members: Map<String, Json> = i
                    .members
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
                ObjectWriter::new("struct")
                    .set("members", members)
                    .opt("optional", i.optional.clone())
                    .finish(&i.extra)
            }
            Self::Command(i) => ObjectWriter::new("command")
                .opt("argument", i.argument.as_ref().map(|a| a.to_json()))
                .opt("result", i.result.as_ref().map(|r| r.to_json()))
                .finish(&i.extra),
        }
    }

    /// The SECoP type name, e.g. `"double"`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Double(_) => "double",
            Self::Scaled(_) => "scaled",
            Self::Int(_) => "int",
            Self::Bool(_) => "bool",
            Self::Enum(_) => "enum",
            Self::String(_) => "string",
            Self::Blob(_) => "blob",
            Self::Array(_) => "array",
            Self::Tuple(_) => "tuple",
            Self::Struct(_) => "struct",
            Self::Command(_) => "command",
        }
    }

    /// Properties not defined by SECoP for this type (custom or from newer versions).
    pub fn extra(&self) -> &Map<String, Json> {
        match self {
            Self::Double(i) => &i.extra,
            Self::Scaled(i) => &i.extra,
            Self::Int(i) => &i.extra,
            Self::Bool(i) => &i.extra,
            Self::Enum(i) => &i.extra,
            Self::String(i) => &i.extra,
            Self::Blob(i) => &i.extra,
            Self::Array(i) => &i.extra,
            Self::Tuple(i) => &i.extra,
            Self::Struct(i) => &i.extra,
            Self::Command(i) => &i.extra,
        }
    }
}

impl DataInfo {
    /// Compares type and SECoP-defined properties, ignoring unknown and custom ones at
    /// every nesting level. Used to check that a postfixed parameter matches its parent.
    pub fn matches(&self, other: &DataInfo) -> bool {
        self.without_extra() == other.without_extra()
    }

    /// A copy with all unknown and custom properties removed, recursively.
    fn without_extra(&self) -> DataInfo {
        let mut copy = self.clone();
        copy.clear_extra();
        copy
    }

    fn clear_extra(&mut self) {
        match self {
            Self::Double(i) => i.extra.clear(),
            Self::Scaled(i) => i.extra.clear(),
            Self::Int(i) => i.extra.clear(),
            Self::Bool(i) => i.extra.clear(),
            Self::Enum(i) => i.extra.clear(),
            Self::String(i) => i.extra.clear(),
            Self::Blob(i) => i.extra.clear(),
            Self::Array(i) => {
                i.extra.clear();
                i.members.clear_extra();
            }
            Self::Tuple(i) => {
                i.extra.clear();
                i.members.iter_mut().for_each(Self::clear_extra);
            }
            Self::Struct(i) => {
                i.extra.clear();
                i.members.values_mut().for_each(Self::clear_extra);
            }
            Self::Command(i) => {
                i.extra.clear();
                i.argument.iter_mut().for_each(|a| a.clear_extra());
                i.result.iter_mut().for_each(|r| r.clear_extra());
            }
        }
    }
}

impl fmt::Display for DataInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_json())
    }
}

impl FromStr for DataInfo {
    type Err = DataInfoError;

    fn from_str(s: &str) -> Result<Self> {
        let json: Json = serde_json::from_str(s)
            .map_err(|e| DataInfoError::new(DataInfoErrorKind::InvalidJson(e.to_string())))?;
        Self::from_json(&json)
    }
}

impl Serialize for DataInfo {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DataInfo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let json = Json::deserialize(deserializer)?;
        Self::from_json(&json).map_err(serde::de::Error::custom)
    }
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
    /// specification says `%.6g` instead. must follows the prose.
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

// ---------------------------------------------------------------------------------------
// building in code

/// Builds a datainfo in code. [`build`](Self::build) runs the same validation as
/// [`DataInfo::from_json`] on the equivalent JSON, and reports the same errors.
///
/// ```
/// use must_types::DataInfo;
///
/// let info = DataInfo::double().min(0.0).max(300.0).unit("K").build()?;
/// assert_eq!(info.to_string(), r#"{"type":"double","min":0.0,"max":300.0,"unit":"K"}"#);
/// # Ok::<(), must_types::DataInfoError>(())
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct DataInfoBuilder(Map<String, Json>);

impl DataInfoBuilder {
    fn new(ty: &str) -> Self {
        let mut map = Map::new();
        map.insert("type".into(), ty.into());
        Self(map)
    }

    /// Sets any property, SECoP-defined or custom.
    pub fn prop(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.0.insert(key.to_owned(), value.into());
        self
    }
    /// Sets `min`.
    pub fn min(self, min: impl Into<Json>) -> Self {
        self.prop("min", min)
    }
    /// Sets `max`.
    pub fn max(self, max: impl Into<Json>) -> Self {
        self.prop("max", max)
    }
    /// Sets `unit`.
    pub fn unit(self, unit: &str) -> Self {
        self.prop("unit", unit)
    }
    /// Sets `fmtstr`.
    pub fn fmtstr(self, fmtstr: &str) -> Self {
        self.prop("fmtstr", fmtstr)
    }
    /// Sets the struct `optional` list.
    pub fn optional<'a>(self, names: impl IntoIterator<Item = &'a str>) -> Self {
        let names: Vec<Json> = names.into_iter().map(Json::from).collect();
        self.prop("optional", names)
    }
    /// Sets a command `argument`.
    pub fn argument(self, argument: &DataInfo) -> Self {
        self.prop("argument", argument.to_json())
    }
    /// Sets a command `result`.
    pub fn result(self, result: &DataInfo) -> Self {
        self.prop("result", result.to_json())
    }
    /// Validates and returns the datainfo.
    pub fn build(self) -> Result<DataInfo> {
        DataInfo::from_json(&Json::Object(self.0))
    }
}

impl DataInfo {
    /// Starts a `double` datainfo.
    pub fn double() -> DataInfoBuilder {
        DataInfoBuilder::new("double")
    }
    /// Starts a `scaled` datainfo.
    pub fn scaled(scale: f64, min: i64, max: i64) -> DataInfoBuilder {
        DataInfoBuilder::new("scaled")
            .prop("scale", scale)
            .min(min)
            .max(max)
    }
    /// Starts an `int` datainfo.
    pub fn int(min: i64, max: i64) -> DataInfoBuilder {
        DataInfoBuilder::new("int").min(min).max(max)
    }
    /// Starts a `bool` datainfo.
    pub fn bool() -> DataInfoBuilder {
        DataInfoBuilder::new("bool")
    }
    /// Starts an `enum` datainfo with the given members.
    pub fn enumeration<'a>(members: impl IntoIterator<Item = (&'a str, i64)>) -> DataInfoBuilder {
        let members: Map<String, Json> = members
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect();
        DataInfoBuilder::new("enum").prop("members", members)
    }
    /// Starts a `string` datainfo.
    pub fn string() -> DataInfoBuilder {
        DataInfoBuilder::new("string")
    }
    /// Starts a `blob` datainfo.
    pub fn blob(maxbytes: u64) -> DataInfoBuilder {
        DataInfoBuilder::new("blob").prop("maxbytes", maxbytes)
    }
    /// Starts an `array` datainfo.
    pub fn array(members: &DataInfo, maxlen: u64) -> DataInfoBuilder {
        DataInfoBuilder::new("array")
            .prop("members", members.to_json())
            .prop("maxlen", maxlen)
    }
    /// Starts a `tuple` datainfo.
    pub fn tuple<'a>(members: impl IntoIterator<Item = &'a DataInfo>) -> DataInfoBuilder {
        let members: Vec<Json> = members.into_iter().map(Self::to_json).collect();
        DataInfoBuilder::new("tuple").prop("members", members)
    }
    /// Starts a `struct` datainfo.
    pub fn structure<'a>(
        members: impl IntoIterator<Item = (&'a str, &'a DataInfo)>,
    ) -> DataInfoBuilder {
        let members: Map<String, Json> = members
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_json()))
            .collect();
        DataInfoBuilder::new("struct").prop("members", members)
    }
    /// Starts a `command` datainfo.
    pub fn command() -> DataInfoBuilder {
        DataInfoBuilder::new("command")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(clippy::needless_pass_by_value)]
    fn parse(json: Json) -> Result<DataInfo> {
        DataInfo::from_json(&json)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn kind(json: Json) -> DataInfoErrorKind {
        parse(json).expect_err("should fail").kind
    }

    // --- 3.1 type dispatch

    #[test]
    fn known_type() {
        let DataInfo::Double(d) =
            parse(json!({"type": "double", "min": 0, "max": 100, "unit": "K"})).unwrap()
        else {
            panic!("not a double")
        };
        assert_eq!(
            (d.min(), d.max(), d.unit()),
            (Some(0.0), Some(100.0), Some("K"))
        );
    }

    #[test]
    fn unknown_type() {
        assert_eq!(
            kind(json!({"type": "float"})),
            DataInfoErrorKind::UnknownType("float".into())
        );
    }

    #[test]
    fn matrix_not_supported() {
        let e =
            parse(json!({"type": "matrix", "elementtype": "<f4", "names": ["x"], "maxlen": [100]}))
                .unwrap_err();
        assert_eq!(e.kind, DataInfoErrorKind::UnsupportedType("matrix".into()));
        assert!(e.to_string().contains("not supported"));
    }

    #[test]
    fn not_an_object() {
        assert_eq!(kind(json!("double")), DataInfoErrorKind::NotAnObject);
        assert_eq!(kind(json!([])), DataInfoErrorKind::NotAnObject);
        assert_eq!(kind(json!({"min": 0})), DataInfoErrorKind::MissingType);
        assert_eq!(kind(json!({"type": 3})), DataInfoErrorKind::MissingType);
    }

    #[test]
    fn missing_mandatory_properties() {
        assert_eq!(
            kind(json!({"type": "int", "min": 0})),
            DataInfoErrorKind::MissingProperty("max")
        );
        assert_eq!(
            kind(json!({"type": "blob"})),
            DataInfoErrorKind::MissingProperty("maxbytes")
        );
        assert_eq!(
            kind(json!({"type": "scaled", "min": 0, "max": 1})),
            DataInfoErrorKind::MissingProperty("scale")
        );
        for ty in ["enum", "tuple", "struct"] {
            assert_eq!(
                kind(json!({"type": ty})),
                DataInfoErrorKind::MissingProperty("members")
            );
        }
        assert_eq!(
            kind(json!({"type": "array", "maxlen": 3})),
            DataInfoErrorKind::MissingProperty("members")
        );
        assert_eq!(
            kind(json!({"type": "array", "members": {"type": "bool"}})),
            DataInfoErrorKind::MissingProperty("maxlen")
        );
    }

    #[test]
    fn property_types() {
        assert!(matches!(
            kind(json!({"type": "int", "min": 0.5, "max": 10})),
            DataInfoErrorKind::WrongPropertyType {
                property: "min",
                ..
            }
        ));
        assert!(matches!(
            kind(json!({"type": "string", "maxchars": -1})),
            DataInfoErrorKind::WrongPropertyType {
                property: "maxchars",
                ..
            }
        ));
        assert!(matches!(
            kind(json!({"type": "double", "unit": 5})),
            DataInfoErrorKind::WrongPropertyType {
                property: "unit",
                ..
            }
        ));
        assert!(matches!(
            kind(json!({"type": "double", "fmtstr": 3})),
            DataInfoErrorKind::WrongPropertyType {
                property: "fmtstr",
                ..
            }
        ));
        // integral floats are accepted for integer properties
        assert!(parse(json!({"type": "int", "min": 0.0, "max": 10.0})).is_ok());
    }

    // --- 3.2 numeric types

    #[test]
    fn consistent_limits() {
        assert_eq!(
            kind(json!({"type": "double", "min": 10, "max": 0})),
            DataInfoErrorKind::InvertedLimits {
                lower: "min",
                upper: "max"
            }
        );
        assert!(parse(json!({"type": "int", "min": 5, "max": 5})).is_ok());
        assert!(parse(json!({"type": "scaled", "scale": 1, "min": 3, "max": 2})).is_err());
    }

    #[test]
    fn scale_factor() {
        for scale in [json!(0), json!(-0.1)] {
            assert_eq!(
                kind(json!({"type": "scaled", "scale": scale, "min": 0, "max": 10})),
                DataInfoErrorKind::InvalidScale
            );
        }
    }

    #[test]
    fn fmtstr_syntax() {
        for f in ["%.3f", "%.12g", "%.0e"] {
            assert!(parse(json!({"type": "double", "fmtstr": f})).is_ok(), "{f}");
        }
        for f in ["%5.3f", "%d", "%.3", "%.05f", "%.123f", ".3f"] {
            assert_eq!(
                kind(json!({"type": "double", "fmtstr": f})),
                DataInfoErrorKind::InvalidFmtstr(f.into()),
                "{f}"
            );
        }
    }

    #[test]
    fn double_defaults() {
        let DataInfo::Double(d) = parse(json!({"type": "double"})).unwrap() else {
            panic!()
        };
        assert_eq!(d.fmtstr(), "%.6g");
        assert!((d.absolute_resolution()).abs() < f64::EPSILON);
        assert!((d.relative_resolution() - 1.2e-7).abs() < f64::EPSILON);
    }

    #[test]
    fn scaled_defaults() {
        let DataInfo::Scaled(s) =
            parse(json!({"type": "scaled", "scale": 0.01, "min": 0, "max": 1000})).unwrap()
        else {
            panic!()
        };
        assert_eq!(s.fmtstr(), "%.2f");
        assert!((s.absolute_resolution() - 0.01).abs() < f64::EPSILON);
        assert!((s.relative_resolution() - 1.2e-7).abs() < f64::EPSILON);

        let fmt = |scale: f64| {
            let DataInfo::Scaled(s) = DataInfo::scaled(scale, 0, 1).build().unwrap() else {
                panic!()
            };
            s.fmtstr()
        };
        assert_eq!(fmt(0.1), "%.1f");
        assert_eq!(fmt(0.001), "%.3f");
        assert_eq!(fmt(0.5), "%.1f");
        assert_eq!(fmt(1.0), "%.0f");
        assert_eq!(fmt(25.0), "%.0f");
    }

    // --- 3.3 enum, string, blob

    #[test]
    fn enum_duplicate_value() {
        assert_eq!(
            kind(json!({"type": "enum", "members": {"on": 1, "active": 1}})),
            DataInfoErrorKind::DuplicateEnumValue {
                value: 1,
                first: "on".into(),
                second: "active".into()
            }
        );
    }

    #[test]
    fn enum_lookup_both_directions() {
        let DataInfo::Enum(e) =
            parse(json!({"type": "enum", "members": {"IDLE": 100, "BUSY": 300}})).unwrap()
        else {
            panic!()
        };
        assert_eq!(e.value_of("BUSY"), Some(300));
        assert_eq!(e.name_of(100), Some("IDLE"));
        assert_eq!(e.name_of(200), None);
    }

    #[test]
    fn enum_names() {
        // names need not be identifiers, but must be non-empty and unique ignoring case
        assert!(parse(json!({"type": "enum", "members": {"heater on": 1, "1st": 2}})).is_ok());
        assert_eq!(
            kind(json!({"type": "enum", "members": {"": 1}})),
            DataInfoErrorKind::EmptyMemberName
        );
        assert!(matches!(
            kind(json!({"type": "enum", "members": {"On": 1, "on": 2}})),
            DataInfoErrorKind::MemberNameClash(_)
        ));
        assert!(parse(json!({"type": "enum", "members": {}})).is_ok());
        assert!(parse(json!({"type": "enum", "members": {"a": 1.5}})).is_err());
    }

    #[test]
    fn length_bounds() {
        assert!(parse(json!({"type": "string", "minchars": 5, "maxchars": 3})).is_err());
        assert!(parse(json!({"type": "blob", "minbytes": 5, "maxbytes": 3})).is_err());
        let DataInfo::String(s) = parse(json!({"type": "string"})).unwrap() else {
            panic!()
        };
        assert_eq!((s.minchars(), s.maxchars(), s.is_utf8()), (0, None, false));
    }

    #[test]
    fn large_maxbytes_on_any_platform() {
        let big = u64::from(u32::MAX) + 1;
        let DataInfo::Blob(b) = parse(json!({"type": "blob", "maxbytes": big})).unwrap() else {
            panic!()
        };
        assert_eq!(b.maxbytes(), big);
    }

    // --- 3.4 structured types

    #[test]
    fn nested_datainfo() {
        let DataInfo::Array(a) = parse(
            json!({"type": "array", "maxlen": 10, "members": {"type": "int", "min": 0, "max": 9}}),
        )
        .unwrap() else {
            panic!()
        };
        assert!(matches!(a.members(), DataInfo::Int(_)));
        assert_eq!((a.minlen(), a.maxlen()), (0, 10));
    }

    #[test]
    fn struct_member_names_clash() {
        let e = parse(json!({"type": "struct", "members": {"X": {"type": "double"}, "x": {"type": "double"}}}))
            .unwrap_err();
        assert!(e.to_string().contains("`x`"), "{e}");
    }

    #[test]
    fn nested_command() {
        let e = parse(json!({"type": "tuple", "members": [{"type": "command"}]})).unwrap_err();
        assert_eq!(e.kind, DataInfoErrorKind::NestedCommand);
        assert_eq!(e.path.to_string(), "[0]");
        assert!(
            parse(json!({"type": "array", "maxlen": 1, "members": {"type": "command"}})).is_err()
        );
        assert!(parse(json!({"type": "command", "argument": {"type": "command"}})).is_err());
    }

    #[test]
    fn nested_error_path() {
        let e = parse(json!({"type": "struct", "members": {"p": {"type": "tuple", "members": [{"type": "bool"}, {"type": "int"}]}}}))
            .unwrap_err();
        assert_eq!(e.kind, DataInfoErrorKind::MissingProperty("min"));
        assert_eq!(e.to_string(), "p[1]: missing mandatory property `min`");
    }

    #[test]
    fn unknown_optional_member() {
        assert_eq!(
            kind(
                json!({"type": "struct", "members": {"x": {"type": "double"}}, "optional": ["y"]})
            ),
            DataInfoErrorKind::UnknownOptionalMember("y".into())
        );
        let DataInfo::Struct(s) = parse(
            json!({"type": "struct", "members": {"x": {"type": "double"}}, "optional": ["x"]}),
        )
        .unwrap() else {
            panic!()
        };
        assert!(s.is_optional("x"));
    }

    #[test]
    fn empty_tuple_and_struct_accepted() {
        assert!(parse(json!({"type": "tuple", "members": []})).is_ok());
        assert!(parse(json!({"type": "struct", "members": {}})).is_ok());
    }

    #[test]
    fn command_without_argument() {
        let DataInfo::Command(c) =
            parse(json!({"type": "command", "result": {"type": "bool"}})).unwrap()
        else {
            panic!()
        };
        assert!(c.argument().is_none());
        assert!(matches!(c.result(), Some(DataInfo::Bool(_))));
        let DataInfo::Command(c) = parse(json!({"type": "command", "argument": null})).unwrap()
        else {
            panic!()
        };
        assert!(c.argument().is_none());
    }

    // --- 3.5 serialization and builders

    #[test]
    fn custom_property_round_trip() {
        let info = parse(json!({"type": "bool", "_note": "x"})).unwrap();
        assert_eq!(info.to_json(), json!({"type": "bool", "_note": "x"}));
        // a property known for another type is preserved as unknown, too
        let info = parse(json!({"type": "bool", "unit": "K"})).unwrap();
        assert_eq!(info.extra().get("unit"), Some(&json!("K")));
    }

    #[test]
    fn round_trip_keeps_order() {
        let info = parse(
            json!({"type": "struct", "members": {"y": {"type": "double"}, "x": {"type": "bool"}}}),
        )
        .unwrap();
        let text = info.to_string();
        assert!(
            text.find("\"y\"").unwrap() < text.find("\"x\"").unwrap(),
            "{text}"
        );
        assert_eq!(text.parse::<DataInfo>().unwrap(), info);
    }

    #[test]
    fn no_defaults_added() {
        let info = parse(json!({"type": "string"})).unwrap();
        assert_eq!(info.to_json(), json!({"type": "string"}));
    }

    #[test]
    fn built_in_code_same_error() {
        let built = DataInfo::int(10, 0).build().unwrap_err();
        let parsed = parse(json!({"type": "int", "min": 10, "max": 0})).unwrap_err();
        assert_eq!(built, parsed);
    }

    #[test]
    fn builders() {
        let status = DataInfo::tuple(&[
            DataInfo::enumeration([("IDLE", 100), ("BUSY", 300)])
                .build()
                .unwrap(),
            DataInfo::string().build().unwrap(),
        ])
        .build()
        .unwrap();
        assert_eq!(
            status.to_json(),
            json!({"type": "tuple", "members": [
                {"type": "enum", "members": {"IDLE": 100, "BUSY": 300}},
                {"type": "string"}
            ]})
        );
        let double = DataInfo::double().build().unwrap();
        let s = DataInfo::structure([("x", &double), ("t", &double)])
            .optional(["t"])
            .build()
            .unwrap();
        assert!(matches!(&s, DataInfo::Struct(i) if i.is_optional("t")));
        let cmd = DataInfo::command()
            .argument(&s)
            .result(&DataInfo::bool().build().unwrap())
            .build();
        assert!(cmd.is_ok());
    }

    #[test]
    fn serde_integration() {
        let info: DataInfo =
            serde_json::from_str(r#"{"type": "int", "min": 0, "max": 3}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"type":"int","min":0,"max":3}"#
        );
        assert!(serde_json::from_str::<DataInfo>(r#"{"type": "int"}"#).is_err());
    }
}
