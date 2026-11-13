// SPDX-License-Identifier: LGPL-3.0-or-later

//! Encoding values to JSON.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value as Json};

use super::{Mode, Result, Value, ValueError};
use crate::datainfo::DataInfo;
use crate::path::PathSegment;

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
