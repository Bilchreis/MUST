// SPDX-License-Identifier: LGPL-3.0-or-later

//! Serializing datainfos to JSON.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json};

use super::{DataInfo, DataInfoError, DataInfoErrorKind, Result};

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
            Self::Matrix(i) => ObjectWriter::new("matrix")
                .set("elementtype", i.elementtype.to_string())
                .set("names", i.names.clone())
                .set("maxlen", i.maxlen.clone())
                .finish(&i.extra),
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
            Self::Matrix(_) => "matrix",
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
            Self::Matrix(i) => &i.extra,
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
            Self::Matrix(i) => i.extra.clear(),
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
