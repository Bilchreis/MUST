// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parsing each type of datainfo from its JSON properties.

use indexmap::IndexMap;
use serde_json::{Map, Value as Json};

use super::{
    ArrayInfo, BlobInfo, BoolInfo, CommandInfo, DataInfo, DataInfoErrorKind, DoubleInfo, EnumInfo,
    IntInfo, MatrixInfo, Props, Result, ScaledInfo, StringInfo, StructInfo, TupleInfo,
    add_member_name, check_bounds, err, json_to_i64, json_to_u64, parse_nested, wrong,
};
use crate::identifier::NameScope;
use crate::path::PathSegment;

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
            "matrix" => MatrixInfo::parse(obj).map(Self::Matrix),
            "command" => CommandInfo::parse(obj).map(Self::Command),
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

impl MatrixInfo {
    const KNOWN: &[&str] = &["elementtype", "names", "maxlen", "compression"];

    fn parse(obj: &Map<String, Json>) -> Result<Self> {
        const NAMES: &str = "a non-empty list of distinct strings";
        const MAXLEN: &str = "a list of one positive integer per name";
        const ELEMENTTYPE: &str = "`<` or `>`, then `i`, `u` or `f`, then 1, 2, 4 or 8 (not `f1`)";
        let p = Props {
            obj,
            known: Self::KNOWN,
        };
        if p.get("compression").is_some() {
            return err(DataInfoErrorKind::UnsupportedProperty("compression"));
        }

        let Some(json_names) = p.get("names") else {
            return err(DataInfoErrorKind::MissingProperty("names"));
        };
        let Json::Array(json_names) = json_names else {
            return Err(wrong("names", NAMES));
        };
        let mut names: Vec<String> = Vec::with_capacity(json_names.len());
        for name in json_names {
            match name {
                Json::String(name) if !names.contains(name) => names.push(name.clone()),
                _ => return Err(wrong("names", NAMES)),
            }
        }
        if names.is_empty() {
            return Err(wrong("names", NAMES));
        }

        let Some(json_maxlen) = p.get("maxlen") else {
            return err(DataInfoErrorKind::MissingProperty("maxlen"));
        };
        let Json::Array(json_maxlen) = json_maxlen else {
            return Err(wrong("maxlen", MAXLEN));
        };
        let maxlen = json_maxlen
            .iter()
            .map(|m| json_to_u64(m).filter(|&m| m > 0))
            .collect::<Option<Vec<_>>>()
            .filter(|m| m.len() == names.len())
            .ok_or_else(|| wrong("maxlen", MAXLEN))?;

        let elementtype = p
            .required("elementtype", Props::string)?
            .parse()
            .map_err(|_| wrong("elementtype", ELEMENTTYPE))?;

        Ok(Self {
            elementtype,
            names,
            maxlen,
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
