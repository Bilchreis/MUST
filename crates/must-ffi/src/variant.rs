// SPDX-License-Identifier: LGPL-3.0-or-later

//! The values behind `CSECoPbaseType*` handles: a SECoP data type together with a value,
//! modelled after SHALL's `CSECoPbaseType` classes.
//!
//! Unlike `must-types`, these are lenient and mutable, as SHALL's are: a description may
//! omit `min`/`max` of an `int` (SECoP 1.0), a value can be created from bare JSON
//! without any description, and limits, scales, enum members and array lengths can be
//! changed after creation. [`Node::to_datainfo`] converts to a validated SECoP 2.0
//! datainfo where a handle meets the SEC node.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use must_types::{DataInfo, Mode, Value};
use serde_json::{Map, Value as Json, json};

/// `SECoP_V_type` codes of SHALL (all listed; array codes are computed as scalar + 10).
#[allow(missing_docs, dead_code)]
pub mod vt {
    pub const NONE: u32 = 0;
    pub const DOUBLE: u32 = 1;
    pub const INTEGER: u32 = 2;
    pub const BOOL: u32 = 3;
    pub const ENUM: u32 = 4;
    pub const SCALED: u32 = 5;
    pub const ARRAY: u32 = 10;
    pub const ARRAY_DOUBLE: u32 = 11;
    pub const ARRAY_INTEGER: u32 = 12;
    pub const ARRAY_BOOL: u32 = 13;
    pub const ARRAY_ENUM: u32 = 14;
    pub const ARRAY_SCALED: u32 = 15;
    pub const STRING: u32 = 20;
    pub const BLOB: u32 = 21;
    pub const STRUCT: u32 = 30;
    pub const TUPLE: u32 = 31;
    pub const COMMAND: u32 = 40;
}

/// `SECoP_V_compareResult` codes of SHALL.
#[allow(missing_docs)]
pub mod vc {
    pub const EQUAL: i32 = 0;
    pub const SIMILAR_VALUE: i32 = 1;
    pub const DIFF_VALUE: i32 = 2;
    pub const DIFF_TYPE: i32 = 3;
}

/// Range used for `int` limits a description leaves out when a SECoP 2.0 datainfo needs
/// them: the 24-bit range the spec recommends (and Frappy uses).
const DEFAULT_INT_LIMIT: i64 = 1 << 24;

/// A struct member or enum member name, kept NUL-terminated so that C callers can get
/// a `const char*` to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(std::ffi::CString);

impl Name {
    /// A name; `None` if it contains a NUL character.
    pub fn new(name: &str) -> Option<Self> {
        std::ffi::CString::new(name).ok().map(Self)
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        self.0.to_str().unwrap_or_default()
    }

    /// The name as a C string, valid as long as this `Name`.
    pub fn as_ptr(&self) -> *const std::ffi::c_char {
        self.0.as_ptr()
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The type of a single numeric value, with its limits.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// `double`; NaN limits mean "no limit".
    Double { min: f64, max: f64 },
    /// `int`.
    Int { min: i64, max: i64 },
    /// `bool`, stored as 0/1.
    Bool,
    /// `enum`, members in declaration order.
    Enum { members: Vec<(Name, i64)> },
    /// `scaled`: transported integer with limits, physical value = integer × scale.
    Scaled { scale: f64, min: i64, max: i64 },
}

impl Scalar {
    fn int_unlimited() -> Self {
        Self::Int {
            min: i64::MIN,
            max: i64::MAX,
        }
    }

    fn code(&self) -> u32 {
        match self {
            Self::Double { .. } => vt::DOUBLE,
            Self::Int { .. } => vt::INTEGER,
            Self::Bool => vt::BOOL,
            Self::Enum { .. } => vt::ENUM,
            Self::Scaled { .. } => vt::SCALED,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Double { .. } => "double",
            Self::Int { .. } => "int",
            Self::Bool => "bool",
            Self::Enum { .. } => "enum",
            Self::Scaled { .. } => "scaled",
        }
    }

    /// Whether `v` is allowed (SHALL's `isValid`): within the limits, an enum member.
    pub(crate) fn accepts(&self, v: Num) -> bool {
        match (self, v) {
            (Self::Double { min, max }, Num::F(x)) => {
                !x.is_nan() && (min.is_nan() || x >= *min) && (max.is_nan() || x <= *max)
            }
            (Self::Int { min, max } | Self::Scaled { min, max, .. }, Num::I(x)) => {
                x >= *min && x <= *max
            }
            (Self::Bool, Num::I(x)) => x == 0 || x == 1,
            (Self::Enum { members }, Num::I(x)) => members.iter().any(|(_, m)| *m == x),
            _ => false,
        }
    }

    pub(crate) fn default_value(&self) -> Num {
        match self {
            Self::Double { min, max } => Num::F(if !min.is_nan() && *min > 0.0 {
                *min
            } else if !max.is_nan() && *max < 0.0 {
                *max
            } else {
                0.0
            }),
            Self::Int { min, max } | Self::Scaled { min, max, .. } => {
                Num::I(0.clamp(*min, (*max).max(*min)))
            }
            Self::Bool => Num::I(0),
            Self::Enum { members } => Num::I(members.first().map_or(0, |m| m.1)),
        }
    }

    /// Value from JSON; `None` if it does not fit the type (limits are checked separately).
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn parse(&self, json: &Json) -> Option<Num> {
        match self {
            Self::Double { .. } => json.as_f64().map(Num::F),
            Self::Bool => match json {
                Json::Bool(b) => Some(Num::I(i64::from(*b))),
                _ => json_int(json).filter(|i| *i == 0 || *i == 1).map(Num::I),
            },
            _ => json_int(json).map(Num::I),
        }
    }

    fn to_json(&self, v: Num) -> Json {
        match (self, v) {
            (Self::Bool, Num::I(i)) => Json::Bool(i != 0),
            (_, Num::F(f)) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
            (_, Num::I(i)) => Json::from(i),
        }
    }

    /// The datainfo properties of this type (without `type`); `complete` fills limits a
    /// SECoP 2.0 datainfo requires.
    fn type_props(&self, map: &mut Map<String, Json>, complete: bool) {
        let limit = |map: &mut Map<String, Json>, min: i64, max: i64| {
            if min != i64::MIN {
                map.insert("min".into(), min.into());
            } else if complete {
                map.insert("min".into(), (-DEFAULT_INT_LIMIT).into());
            }
            if max != i64::MAX {
                map.insert("max".into(), max.into());
            } else if complete {
                map.insert("max".into(), DEFAULT_INT_LIMIT.into());
            }
        };
        match self {
            Self::Double { min, max } => {
                for (k, v) in [("min", min), ("max", max)] {
                    if let Some(n) = serde_json::Number::from_f64(*v) {
                        map.insert(k.into(), Json::Number(n));
                    }
                }
            }
            Self::Int { min, max } => limit(map, *min, *max),
            Self::Bool => {}
            Self::Enum { members } => {
                let m: Map<String, Json> = members
                    .iter()
                    .map(|(k, v)| (k.to_string(), Json::from(*v)))
                    .collect();
                map.insert("members".into(), m.into());
            }
            Self::Scaled { scale, min, max } => {
                map.insert("scale".into(), json!(scale));
                limit(map, *min, *max);
            }
        }
    }
}

/// A numeric value: `F` for `double`, `I` for all integer-based types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    /// A double.
    F(f64),
    /// An integer (also bool 0/1, enum value, scaled transported integer).
    I(i64),
}

/// One node of a value tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Type and value.
    pub kind: Kind,
    /// Datainfo properties SHALL does not model itself ("additional"), e.g. `unit`.
    pub extra: Map<String, Json>,
}

/// The kinds of values.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// JSON `null` (`SECoP_VT_NONE`).
    Null,
    /// A single number.
    Scalar { ty: Scalar, value: Num },
    /// An array of numbers of one type, addressed as one position.
    ScalarArray {
        ty: Scalar,
        values: Vec<Num>,
        minlen: u32,
        maxlen: u32,
        member_extra: Map<String, Json>,
    },
    /// A string or blob, stored as bytes.
    Bytes {
        blob: bool,
        bytes: Vec<u8>,
        minlen: u32,
        maxlen: u32,
        utf8: Option<bool>,
    },
    /// An array of non-numeric items; `proto` is the item type with a default value.
    Array {
        proto: Box<Node>,
        items: Vec<Node>,
        minlen: u32,
        maxlen: u32,
    },
    /// A tuple.
    Tuple(Vec<Node>),
    /// A struct, members in declaration order.
    Struct {
        items: Vec<(Name, Node)>,
        optional: Vec<String>,
    },
    /// A command's argument and result types.
    Command {
        argument: Option<Box<Node>>,
        result: Option<Box<Node>>,
    },
}

fn json_int(json: &Json) -> Option<i64> {
    if let Some(i) = json.as_i64() {
        return Some(i);
    }
    let f = json.as_f64()?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    (f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64).then_some(f as i64)
}

fn json_u32(json: &Json) -> Option<u32> {
    json.as_u64().and_then(|u| u32::try_from(u).ok())
}

impl Node {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            extra: Map::new(),
        }
    }

    /// A single value of the given scalar type.
    pub fn scalar(ty: Scalar, value: Num) -> Self {
        Self::new(Kind::Scalar { ty, value })
    }

    /// A string value without limits.
    pub fn string(bytes: Vec<u8>) -> Self {
        Self::new(Kind::Bytes {
            blob: false,
            bytes,
            minlen: 0,
            maxlen: u32::MAX,
            utf8: None,
        })
    }

    // --- descriptions

    /// Creates a value with default content from a datainfo description, like SHALL's
    /// `CSECoPbaseType::createSECoP`: type names are case-insensitive, `int` limits may
    /// be missing, unknown properties are kept as additional items.
    #[allow(clippy::too_many_lines, reason = "one arm per datainfo type")]
    pub fn from_description(json: &Json, allow_command: bool) -> Option<Self> {
        let obj = json.as_object()?;
        let ty = obj.get("type")?.as_str()?.to_ascii_lowercase();
        let mut rest = obj.clone();
        rest.shift_remove("type");
        let mut take = |key: &str| rest.shift_remove(key);
        let kind = match ty.as_str() {
            "double" | "int" | "bool" | "enum" | "scaled" => {
                let scalar = parse_scalar(&ty, &mut take)?;
                let value = scalar.default_value();
                Kind::Scalar { ty: scalar, value }
            }
            "string" | "blob" => {
                let blob = ty == "blob";
                let (lo, hi) = if blob {
                    ("minbytes", "maxbytes")
                } else {
                    ("minchars", "maxchars")
                };
                let (minlen, maxlen) = parse_lengths(&mut take, lo, hi)?;
                let utf8 = if blob {
                    None
                } else {
                    match take("isUTF8") {
                        None => None,
                        Some(Json::Bool(b)) => Some(b),
                        Some(_) => return None,
                    }
                };
                Kind::Bytes {
                    blob,
                    bytes: vec![0; minlen as usize],
                    minlen,
                    maxlen,
                    utf8,
                }
            }
            "array" => {
                let members = take("members")?;
                let (minlen, maxlen) = parse_lengths(&mut take, "minlen", "maxlen")?;
                let mobj = members.as_object()?;
                let mty = mobj.get("type")?.as_str()?.to_ascii_lowercase();
                if matches!(mty.as_str(), "double" | "int" | "bool" | "enum" | "scaled") {
                    let mut mrest = mobj.clone();
                    mrest.shift_remove("type");
                    let scalar = parse_scalar(&mty, &mut |k| mrest.shift_remove(k))?;
                    let values = vec![scalar.default_value(); minlen as usize];
                    Kind::ScalarArray {
                        ty: scalar,
                        values,
                        minlen,
                        maxlen,
                        member_extra: mrest,
                    }
                } else {
                    let proto = Box::new(Self::from_description(&members, false)?);
                    let items = vec![(*proto).clone(); minlen as usize];
                    Kind::Array {
                        proto,
                        items,
                        minlen,
                        maxlen,
                    }
                }
            }
            "tuple" => {
                let members = take("members")?;
                let items = members
                    .as_array()?
                    .iter()
                    .map(|m| Self::from_description(m, false))
                    .collect::<Option<Vec<_>>>()?;
                Kind::Tuple(items)
            }
            "struct" => {
                let members = take("members")?;
                let items = members
                    .as_object()?
                    .iter()
                    .map(|(k, m)| Some((Name::new(k)?, Self::from_description(m, false)?)))
                    .collect::<Option<Vec<_>>>()?;
                let optional = match take("optional") {
                    None => Vec::new(),
                    Some(o) => o
                        .as_array()?
                        .iter()
                        .map(|n| n.as_str().map(str::to_owned))
                        .collect::<Option<_>>()?,
                };
                Kind::Struct { items, optional }
            }
            "command" if allow_command => {
                let mut part = |key: &str| -> Option<Option<Box<Self>>> {
                    match take(key) {
                        None | Some(Json::Null) => Some(None),
                        Some(d) => Some(Some(Box::new(Self::from_description(&d, false)?))),
                    }
                };
                let argument = part("argument")?;
                let result = part("result")?;
                Kind::Command { argument, result }
            }
            _ => return None,
        };
        Some(Self { kind, extra: rest })
    }

    /// A value with a type inferred from bare JSON, like SHALL's `importSECoP(json)`.
    /// An empty array has no type and gives `None`.
    pub fn from_json(json: &Json) -> Option<Self> {
        let kind = match json {
            Json::Null => Kind::Null,
            Json::Bool(b) => Kind::Scalar {
                ty: Scalar::Bool,
                value: Num::I(i64::from(*b)),
            },
            Json::Number(n) if n.is_f64() => Kind::Scalar {
                ty: Scalar::Double {
                    min: f64::NAN,
                    max: f64::NAN,
                },
                value: Num::F(n.as_f64()?),
            },
            Json::Number(_) => Kind::Scalar {
                ty: Scalar::int_unlimited(),
                value: Num::I(json_int(json)?),
            },
            Json::String(s) => return Some(Self::string(s.as_bytes().to_vec())),
            Json::Object(o) => Kind::Struct {
                items: o
                    .iter()
                    .map(|(k, v)| Some((Name::new(k)?, Self::from_json(v)?)))
                    .collect::<Option<_>>()?,
                optional: Vec::new(),
            },
            Json::Array(items) => {
                let first = items.first()?;
                let same = |f: fn(&Json) -> bool| items.iter().all(f);
                if same(Json::is_boolean) {
                    let values = items
                        .iter()
                        .map(|b| Num::I(i64::from(b.as_bool().unwrap_or(false))))
                        .collect();
                    scalar_array(Scalar::Bool, values)
                } else if same(|j| j.as_number().is_some_and(serde_json::Number::is_f64)) {
                    let values = items
                        .iter()
                        .map(|f| Num::F(f.as_f64().unwrap_or(0.0)))
                        .collect();
                    scalar_array(
                        Scalar::Double {
                            min: f64::NAN,
                            max: f64::NAN,
                        },
                        values,
                    )
                } else if same(|j| j.is_i64() || j.is_u64()) {
                    let values = items
                        .iter()
                        .map(|i| json_int(i).map(Num::I))
                        .collect::<Option<Vec<_>>>()?;
                    scalar_array(Scalar::int_unlimited(), values)
                } else if same(Json::is_string) {
                    let proto = Box::new(Self::from_json(first)?);
                    let items = items
                        .iter()
                        .map(Self::from_json)
                        .collect::<Option<Vec<_>>>()?;
                    Kind::Array {
                        proto,
                        items,
                        minlen: 0,
                        maxlen: u32::MAX,
                    }
                } else {
                    Kind::Tuple(items.iter().map(Self::from_json).collect::<Option<_>>()?)
                }
            }
        };
        Some(Self::new(kind))
    }

    /// Replaces the value from JSON, keeping the type; with `strict`, limits are checked
    /// too. Returns `false` (leaving the value unspecified) if it does not fit.
    #[allow(clippy::too_many_lines, reason = "one arm per kind")]
    pub fn import(&mut self, json: &Json, strict: bool) -> bool {
        let ok_len = |len: usize, min: u32, max: u32| {
            !strict || (len >= min as usize && len <= max as usize)
        };
        match &mut self.kind {
            Kind::Null => json.is_null(),
            Kind::Scalar { ty, value } => match ty.parse(json) {
                Some(v) if !strict || ty.accepts(v) || matches!(ty, Scalar::Bool) => {
                    *value = v;
                    true
                }
                _ => false,
            },
            Kind::ScalarArray {
                ty,
                values,
                minlen,
                maxlen,
                ..
            } => {
                let Some(items) = json.as_array() else {
                    return false;
                };
                if !ok_len(items.len(), *minlen, *maxlen) {
                    return false;
                }
                let Some(new) = items
                    .iter()
                    .map(|j| ty.parse(j))
                    .collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                if strict && !new.iter().all(|v| ty.accepts(*v)) {
                    return false;
                }
                *values = new;
                true
            }
            Kind::Bytes {
                blob,
                bytes,
                minlen,
                maxlen,
                ..
            } => {
                let Some(s) = json.as_str() else { return false };
                let new = if *blob {
                    match BASE64.decode(s) {
                        Ok(b) => b,
                        Err(_) => return false,
                    }
                } else {
                    s.as_bytes().to_vec()
                };
                let len = if *blob { new.len() } else { s.chars().count() };
                if !ok_len(len, *minlen, *maxlen) {
                    return false;
                }
                *bytes = new;
                true
            }
            Kind::Array {
                proto,
                items,
                minlen,
                maxlen,
            } => {
                let Some(json_items) = json.as_array() else {
                    return false;
                };
                if !ok_len(json_items.len(), *minlen, *maxlen) {
                    return false;
                }
                let mut new = Vec::with_capacity(json_items.len());
                for j in json_items {
                    let mut item = (**proto).clone();
                    if !item.import(j, strict) {
                        return false;
                    }
                    new.push(item);
                }
                *items = new;
                true
            }
            Kind::Tuple(items) => {
                let Some(json_items) = json.as_array() else {
                    return false;
                };
                json_items.len() == items.len()
                    && items
                        .iter_mut()
                        .zip(json_items)
                        .all(|(i, j)| i.import(j, strict))
            }
            Kind::Struct { items, optional } => {
                let Some(obj) = json.as_object() else {
                    return false;
                };
                if obj
                    .keys()
                    .any(|k| !items.iter().any(|(n, _)| n.as_str() == k))
                {
                    return false;
                }
                items
                    .iter_mut()
                    .all(|(name, item)| match obj.get(name.as_str()) {
                        Some(j) => item.import(j, strict),
                        None => !strict || optional.iter().any(|o| o == name.as_str()),
                    })
            }
            Kind::Command { .. } => false,
        }
    }

    // --- export

    /// The value as SECoP JSON (scaled and enum as integers, blob as base64).
    pub fn export_value(&self) -> Json {
        match &self.kind {
            Kind::Null | Kind::Command { .. } => Json::Null,
            Kind::Scalar { ty, value } => ty.to_json(*value),
            Kind::ScalarArray { ty, values, .. } => values.iter().map(|v| ty.to_json(*v)).collect(),
            Kind::Bytes {
                blob: true, bytes, ..
            } => BASE64.encode(bytes).into(),
            Kind::Bytes { bytes, .. } => String::from_utf8_lossy(bytes).into_owned().into(),
            Kind::Array { items, .. } | Kind::Tuple(items) => {
                items.iter().map(Self::export_value).collect()
            }
            Kind::Struct { items, .. } => items
                .iter()
                .map(|(k, v)| (k.to_string(), v.export_value()))
                .collect::<Map<_, _>>()
                .into(),
        }
    }

    /// The datainfo of the type with the additional items, as SHALL's `exportType`;
    /// `None` for a value without type (`null`).
    pub fn export_type(&self) -> Option<Json> {
        self.type_json(false)
    }

    /// A SECoP 2.0 datainfo for the type, with limits the 2.0 spec requires filled in.
    #[allow(dead_code, reason = "used by the server ABI's addParameter")]
    pub fn to_datainfo(&self) -> Option<DataInfo> {
        DataInfo::from_json(&self.type_json(true)?).ok()
    }

    /// The value as a `must-types` value of `info`.
    #[allow(dead_code, reason = "used by the server ABI's updateParameter")]
    pub fn to_value(&self, info: &DataInfo) -> Option<Value> {
        info.decode(&self.export_value(), Mode::Report).ok()
    }

    fn type_json(&self, complete: bool) -> Option<Json> {
        let mut map = Map::new();
        let lengths = |map: &mut Map<String, Json>, lo: &str, hi: &str, min: u32, max: u32| {
            if min > 0 {
                map.insert(lo.into(), min.into());
            }
            if max != u32::MAX || complete {
                map.insert(hi.into(), max.into());
            }
        };
        match &self.kind {
            Kind::Null => return None,
            Kind::Scalar { ty, .. } => {
                map.insert("type".into(), ty.name().into());
                ty.type_props(&mut map, complete);
            }
            Kind::ScalarArray {
                ty,
                minlen,
                maxlen,
                member_extra,
                ..
            } => {
                let mut members = Map::new();
                members.insert("type".into(), ty.name().into());
                ty.type_props(&mut members, complete);
                for (k, v) in member_extra {
                    members.entry(k.clone()).or_insert_with(|| v.clone());
                }
                map.insert("type".into(), "array".into());
                map.insert("members".into(), members.into());
                lengths(&mut map, "minlen", "maxlen", *minlen, *maxlen);
            }
            Kind::Bytes {
                blob,
                minlen,
                maxlen,
                utf8,
                ..
            } => {
                if *blob {
                    map.insert("type".into(), "blob".into());
                    lengths(&mut map, "minbytes", "maxbytes", *minlen, *maxlen);
                } else {
                    map.insert("type".into(), "string".into());
                    if *minlen > 0 {
                        map.insert("minchars".into(), (*minlen).into());
                    }
                    if *maxlen != u32::MAX {
                        map.insert("maxchars".into(), (*maxlen).into());
                    }
                    if let Some(u) = utf8 {
                        map.insert("isUTF8".into(), (*u).into());
                    } else if complete {
                        // SHALL accepted any UTF-8 text
                        map.insert("isUTF8".into(), true.into());
                    }
                }
            }
            Kind::Array {
                proto,
                minlen,
                maxlen,
                ..
            } => {
                map.insert("type".into(), "array".into());
                map.insert("members".into(), proto.type_json(complete)?);
                lengths(&mut map, "minlen", "maxlen", *minlen, *maxlen);
            }
            Kind::Tuple(items) => {
                map.insert("type".into(), "tuple".into());
                let members = items
                    .iter()
                    .map(|i| i.type_json(complete))
                    .collect::<Option<Vec<_>>>()?;
                map.insert("members".into(), members.into());
            }
            Kind::Struct { items, optional } => {
                map.insert("type".into(), "struct".into());
                let members = items
                    .iter()
                    .map(|(k, v)| Some((k.to_string(), v.type_json(complete)?)))
                    .collect::<Option<Map<_, _>>>()?;
                map.insert("members".into(), members.into());
                if !optional.is_empty() {
                    map.insert("optional".into(), optional.clone().into());
                }
            }
            Kind::Command { argument, result } => {
                map.insert("type".into(), "command".into());
                if let Some(a) = argument {
                    map.insert("argument".into(), a.type_json(complete)?);
                }
                if let Some(r) = result {
                    map.insert("result".into(), r.type_json(complete)?);
                }
            }
        }
        for (k, v) in &self.extra {
            map.entry(k.clone()).or_insert_with(|| v.clone());
        }
        Some(map.into())
    }

    // --- positions

    /// `SECoP_V_type` code of this node.
    pub fn code(&self) -> u32 {
        match &self.kind {
            Kind::Null => vt::NONE,
            Kind::Scalar { ty, .. } => ty.code(),
            Kind::ScalarArray { ty, .. } => ty.code() + 10,
            Kind::Bytes { blob: false, .. } => vt::STRING,
            Kind::Bytes { blob: true, .. } => vt::BLOB,
            Kind::Array { .. } => vt::ARRAY,
            Kind::Tuple(_) => vt::TUPLE,
            Kind::Struct { .. } => vt::STRUCT,
            Kind::Command { .. } => vt::COMMAND,
        }
    }

    /// The count `SECoP_V_getInfo` reports: items of containers, elements of number
    /// arrays, bytes of strings, 2 for commands, 1 otherwise.
    pub fn count(&self) -> usize {
        match &self.kind {
            Kind::Struct { items, .. } => items.len(),
            Kind::Tuple(items) | Kind::Array { items, .. } => items.len(),
            Kind::ScalarArray { values, .. } => values.len(),
            Kind::Bytes { bytes, .. } => bytes.len(),
            Kind::Command { .. } => 2,
            Kind::Null | Kind::Scalar { .. } => 1,
        }
    }

    fn children(&self) -> Vec<(Option<&str>, &Node)> {
        match &self.kind {
            Kind::Struct { items, .. } => {
                items.iter().map(|(k, v)| (Some(k.as_str()), v)).collect()
            }
            Kind::Tuple(items) | Kind::Array { items, .. } => {
                items.iter().map(|i| (None, i)).collect()
            }
            Kind::Command { argument, result } => [argument, result]
                .into_iter()
                .flatten()
                .map(|b| (None, &**b))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn child_mut(&mut self, index: usize) -> Option<&mut Node> {
        match &mut self.kind {
            Kind::Struct { items, .. } => items.get_mut(index).map(|(_, v)| v),
            Kind::Tuple(items) | Kind::Array { items, .. } => items.get_mut(index),
            Kind::Command { argument, result } => [argument, result]
                .into_iter()
                .flatten()
                .nth(index)
                .map(|b| &mut **b),
            _ => None,
        }
    }

    /// Finds the node at `position` of a pre-order walk (this node is position 0, then
    /// each child's subtree). Returns the child path and the struct member name of the
    /// target, which SHALL reports only for non-container struct members.
    pub fn locate(&self, position: u32) -> Option<(Vec<usize>, Option<String>)> {
        let mut remaining = position;
        let mut path = Vec::new();
        self.walk(&mut remaining, &mut path, None)
            .map(|name| (path, name))
    }

    /// Walks to position `*remaining`; the outer `None` means not found, the inner one an
    /// item without name.
    #[allow(clippy::option_option)]
    fn walk(
        &self,
        remaining: &mut u32,
        path: &mut Vec<usize>,
        name: Option<&str>,
    ) -> Option<Option<String>> {
        if *remaining == 0 {
            let leaf = self.children().is_empty()
                && !matches!(
                    self.kind,
                    Kind::Struct { .. }
                        | Kind::Tuple(_)
                        | Kind::Array { .. }
                        | Kind::Command { .. }
                );
            return Some(if leaf { name.map(str::to_owned) } else { None });
        }
        *remaining -= 1;
        for (i, (child_name, child)) in self.children().into_iter().enumerate() {
            path.push(i);
            if let Some(found) = child.walk(remaining, path, child_name) {
                return Some(found);
            }
            path.pop();
        }
        // the remaining positions were not inside this subtree; give back the one taken
        // for this node only if nothing below consumed anything (handled by the caller)
        None
    }

    /// The node at `path`.
    pub fn at(&self, path: &[usize]) -> Option<&Node> {
        let mut node = self;
        for &i in path {
            node = *node.children().get(i).map(|(_, n)| n)?;
        }
        Some(node)
    }

    /// The node at `path`, mutable.
    pub fn at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let mut node = self;
        for &i in path {
            node = node.child_mut(i)?;
        }
        Some(node)
    }

    /// Number of positions in this subtree.
    #[cfg(test)]
    pub fn positions(&self) -> u32 {
        1 + self
            .children()
            .iter()
            .map(|(_, c)| c.positions())
            .sum::<u32>()
    }

    // --- comparison

    /// `SECoP_V_compare`: different kinds give `DIFF_TYPE`; the same kind with different
    /// limits, scale, members or lengths `SIMILAR_VALUE` if the values are equal;
    /// otherwise `EQUAL` or `DIFF_VALUE`.
    pub fn compare(&self, other: &Node) -> i32 {
        if !self.same_shape(other) {
            return vc::DIFF_TYPE;
        }
        if self.export_value() != other.export_value() || self.extra != other.extra {
            return vc::DIFF_VALUE;
        }
        if self.export_type() == other.export_type() {
            vc::EQUAL
        } else {
            vc::SIMILAR_VALUE
        }
    }

    /// Same type codes and structure, ignoring limits and other type properties.
    fn same_shape(&self, other: &Node) -> bool {
        if self.code() != other.code() {
            return false;
        }
        match (&self.kind, &other.kind) {
            (Kind::Struct { items: a, .. }, Kind::Struct { items: b, .. }) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((ka, va), (kb, vb))| ka == kb && va.same_shape(vb))
            }
            (Kind::Tuple(a), Kind::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_shape(y))
            }
            (Kind::Array { proto: a, .. }, Kind::Array { proto: b, .. }) => a.same_shape(b),
            _ => true,
        }
    }
}

fn scalar_array(ty: Scalar, values: Vec<Num>) -> Kind {
    Kind::ScalarArray {
        ty,
        values,
        minlen: 0,
        maxlen: u32::MAX,
        member_extra: Map::new(),
    }
}

/// Parses the type properties of a scalar type, taking them out of the description.
fn parse_scalar(ty: &str, take: &mut dyn FnMut(&str) -> Option<Json>) -> Option<Scalar> {
    let int_limit = |v: Option<Json>, default: i64| -> Option<i64> {
        match v {
            None | Some(Json::Null) => Some(default),
            Some(j) => json_int(&j),
        }
    };
    let float_limit = |v: Option<Json>| -> Option<f64> {
        match v {
            None | Some(Json::Null) => Some(f64::NAN),
            Some(j) => j.as_f64(),
        }
    };
    let scalar = match ty {
        "double" => Scalar::Double {
            min: float_limit(take("min"))?,
            max: float_limit(take("max"))?,
        },
        "int" => Scalar::Int {
            min: int_limit(take("min"), i64::MIN)?,
            max: int_limit(take("max"), i64::MAX)?,
        },
        "bool" => Scalar::Bool,
        "enum" => {
            let members = take("members")?;
            let members = members
                .as_object()?
                .iter()
                .map(|(k, v)| Some((Name::new(k)?, json_int(v)?)))
                .collect::<Option<Vec<_>>>()?;
            Scalar::Enum { members }
        }
        "scaled" => {
            let scale = take("scale")?
                .as_f64()
                .filter(|s| s.is_finite() && *s > 0.0)?;
            Scalar::Scaled {
                scale,
                min: int_limit(take("min"), i64::MIN)?,
                max: int_limit(take("max"), i64::MAX)?,
            }
        }
        _ => return None,
    };
    let limits_ok = match &scalar {
        Scalar::Double { min, max } => min.is_nan() || max.is_nan() || min <= max,
        Scalar::Int { min, max } | Scalar::Scaled { min, max, .. } => min <= max,
        _ => true,
    };
    limits_ok.then_some(scalar)
}

fn parse_lengths(
    take: &mut dyn FnMut(&str) -> Option<Json>,
    lo: &str,
    hi: &str,
) -> Option<(u32, u32)> {
    let min = match take(lo) {
        None => 0,
        Some(j) => json_u32(&j)?,
    };
    let max = match take(hi) {
        None => u32::MAX,
        Some(j) => json_u32(&j)?,
    };
    (min <= max).then_some((min, max))
}

// ---------------------------------------------------------------------------------------
// handle table

/// The opaque C type behind `CSECoPbaseType*`.
#[repr(C)]
pub struct CSECoPbaseType {
    _private: [u8; 0],
}

/// All live handles. Every C function checks a handle against this table, as SHALL does,
/// so that stale or foreign pointers are rejected.
///
/// Handles are numbers from a counter, not the nodes' addresses: an address is reused
/// once its node is freed, so a destroyed handle would silently refer to a new value.
/// The counter only repeats after it wraps around (2^29 handles on 32-bit targets), and
/// then skips handles still in use.
static HANDLES: LazyLock<Mutex<HandleTable>> = LazyLock::new(Mutex::default);

/// First handle value; handles step by [`HANDLE_STEP`] so they look like aligned pointers.
const FIRST_HANDLE: usize = 0x1_0000;
const HANDLE_STEP: usize = 8;

struct HandleTable {
    nodes: HashMap<usize, Node>,
    next: usize,
}

impl Default for HandleTable {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            next: FIRST_HANDLE,
        }
    }
}

fn handles() -> MutexGuard<'static, HandleTable> {
    HANDLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl HandleTable {
    /// Stores `node` under the next free handle.
    fn insert(&mut self, node: Node) -> usize {
        let mut id = self.next;
        while self.nodes.contains_key(&id) {
            id = next_handle(id);
        }
        self.next = next_handle(id);
        self.nodes.insert(id, node);
        id
    }
}

/// Registers a node and returns its handle.
pub fn new_handle(node: Node) -> *mut CSECoPbaseType {
    std::ptr::without_provenance_mut(handles().insert(node))
}

fn next_handle(id: usize) -> usize {
    match id.checked_add(HANDLE_STEP) {
        Some(next) => next,
        None => FIRST_HANDLE,
    }
}

/// Runs `f` on the node of a handle; `None` for NULL or unknown handles.
pub fn with_handle<R>(handle: *const CSECoPbaseType, f: impl FnOnce(&mut Node) -> R) -> Option<R> {
    let mut table = handles();
    table.nodes.get_mut(&handle.addr()).map(f)
}

/// A copy of the node of a handle.
pub fn clone_handle(handle: *const CSECoPbaseType) -> Option<Node> {
    with_handle(handle, |n| n.clone())
}

/// Removes and frees a handle; returns whether it existed.
pub fn destroy_handle(handle: *const CSECoPbaseType) -> bool {
    handles().nodes.remove(&handle.addr()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value)]
    fn describe(j: Json) -> Node {
        Node::from_description(&j, false).unwrap()
    }

    #[test]
    fn descriptions() {
        let d = describe(json!({"type": "DOUBLE", "min": 0, "max": 10, "unit": "K"}));
        assert_eq!(d.code(), vt::DOUBLE);
        assert_eq!(d.extra.get("unit"), Some(&json!("K")));
        assert_eq!(
            d.export_type(),
            Some(json!({"type": "double", "min": 0.0, "max": 10.0, "unit": "K"}))
        );

        // SECoP 1.0: int without limits
        let i = describe(json!({"type": "int"}));
        assert_eq!(i.export_type(), Some(json!({"type": "int"})));
        assert_eq!(
            i.to_datainfo().unwrap().to_json(),
            json!({"type": "int", "min": -16_777_216, "max": 16_777_216})
        );

        let a = describe(json!({"type": "array", "members": {"type": "int"}, "maxlen": 5}));
        assert_eq!(
            (a.code(), a.count()),
            (vt::ARRAY_INTEGER, 0),
            "starts at minlen"
        );
        let s = describe(
            json!({"type": "array", "members": {"type": "string"}, "minlen": 2, "maxlen": 5}),
        );
        assert_eq!((s.code(), s.count()), (vt::ARRAY, 2));

        assert!(Node::from_description(&json!({"type": "nope"}), false).is_none());
        assert!(Node::from_description(&json!({"type": "command"}), false).is_none());
        assert!(Node::from_description(&json!({"type": "command"}), true).is_some());
        assert!(
            Node::from_description(&json!({"type": "double", "min": 5, "max": 1}), false).is_none()
        );
    }

    #[test]
    fn inferred_types() {
        let code = |j: Json| Node::from_json(&j).map(|n| n.code());
        assert_eq!(code(json!(true)), Some(vt::BOOL));
        assert_eq!(code(json!(1.5)), Some(vt::DOUBLE));
        assert_eq!(code(json!(3)), Some(vt::INTEGER));
        assert_eq!(code(json!("x")), Some(vt::STRING));
        assert_eq!(code(json!({"a": 1})), Some(vt::STRUCT));
        assert_eq!(code(json!([1.0, 2.5])), Some(vt::ARRAY_DOUBLE));
        assert_eq!(code(json!([1, 2])), Some(vt::ARRAY_INTEGER));
        assert_eq!(code(json!([true])), Some(vt::ARRAY_BOOL));
        assert_eq!(code(json!(["a", "b"])), Some(vt::ARRAY));
        assert_eq!(code(json!([1, "b"])), Some(vt::TUPLE));
        assert_eq!(code(json!(null)), Some(vt::NONE));
        assert_eq!(code(json!([])), None);
    }

    #[test]
    fn import_with_hint_type() {
        let mut s = describe(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
        assert!(s.import(&json!(1255), true));
        assert_eq!(s.export_value(), json!(1255));
        assert!(!s.import(&json!(2501), true));
        let mut b = describe(json!({"type": "blob", "maxbytes": 8}));
        assert!(b.import(&json!("U0VDb1A="), true));
        assert_eq!(b.count(), 5);
        assert_eq!(b.export_value(), json!("U0VDb1A="));
    }

    #[test]
    fn walk_positions() {
        let n = Node::from_json(&json!({"a": 1.0, "b": "x"})).unwrap();
        assert_eq!(n.locate(0), Some((vec![], None)));
        assert_eq!(n.locate(1), Some((vec![0], Some("a".into()))));
        assert_eq!(n.locate(2), Some((vec![1], Some("b".into()))));
        assert_eq!(n.locate(3), None);
        assert_eq!(n.positions(), 3);

        let nested = Node::from_json(&json!({"p": {"x": 1, "y": [1, "z"]}, "q": true})).unwrap();
        // 0 root, 1 p (container: no name), 2 x, 3 y (tuple), 4 1, 5 "z", 6 q
        assert_eq!(nested.locate(1), Some((vec![0], None)));
        assert_eq!(nested.locate(2), Some((vec![0, 0], Some("x".into()))));
        assert_eq!(nested.locate(5), Some((vec![0, 1, 1], None)));
        assert_eq!(nested.locate(6), Some((vec![1], Some("q".into()))));
        assert_eq!(nested.locate(7), None);
    }

    #[test]
    fn comparison() {
        let a = Node::from_json(&json!(1.5)).unwrap();
        let s = Node::from_json(&json!("x")).unwrap();
        assert_eq!(a.compare(&s), vc::DIFF_TYPE);
        assert_eq!(a.compare(&a.clone()), vc::EQUAL);
        let mut b = a.clone();
        b.import(&json!(2.0), false);
        assert_eq!(a.compare(&b), vc::DIFF_VALUE);
        let limited = describe(json!({"type": "double", "min": 0, "max": 10}));
        let mut limited = limited;
        limited.import(&json!(1.5), true);
        assert_eq!(a.compare(&limited), vc::SIMILAR_VALUE);
    }

    #[test]
    fn handle_table() {
        let h = new_handle(Node::from_json(&json!(1)).unwrap());
        assert_eq!(with_handle(h, |n| n.code()), Some(vt::INTEGER));
        assert!(destroy_handle(h));
        assert!(!destroy_handle(h));
        assert_eq!(with_handle(h, |n| n.code()), None);
    }

    #[test]
    fn destroyed_handles_are_not_reused() {
        let h = new_handle(Node::from_json(&json!(1)).unwrap());
        assert!(destroy_handle(h));
        // new values, even when their memory takes the place of the destroyed one
        let others: Vec<_> = (0..1000)
            .map(|_| new_handle(Node::from_json(&json!(2.5)).unwrap()))
            .collect();
        assert!(!others.contains(&h));
        assert_eq!(with_handle(h, |n| n.code()), None);
        for o in others {
            assert!(destroy_handle(o));
        }
    }

    #[test]
    fn handle_counter_wraps_and_skips_live_handles() {
        let node = || Node::from_json(&json!(1)).unwrap();
        let mut table = HandleTable::default();
        let first = table.insert(node());
        assert_eq!(first, FIRST_HANDLE);
        // the counter wraps around at the end of the address range ...
        table.next = usize::MAX - 3;
        assert_eq!(table.insert(node()), usize::MAX - 3);
        // ... and skips handles still in use
        assert_eq!(table.insert(node()), FIRST_HANDLE + HANDLE_STEP);
        assert_eq!(table.nodes.len(), 3);
    }
}
