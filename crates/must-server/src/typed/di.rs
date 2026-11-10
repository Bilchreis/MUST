// SPDX-License-Identifier: LGPL-3.0-or-later

//! Datainfos as the module macro writes them: a SECoP type name and its properties.

use must_types::DataInfo;
use serde_json::{Map, Value as Json};

use super::{STATUS_CODES, SecopType};

/// A datainfo being written: `Di::new("double").set("min", 0.0).set("unit", "K")`.
#[doc(hidden)]
#[derive(Debug, Clone)]
#[must_use]
pub struct Di {
    props: Map<String, Json>,
    /// A problem found while writing it, reported by [`Di::build`].
    problem: Option<String>,
}

impl Di {
    /// A datainfo of the SECoP type `ty`.
    pub fn new(ty: &str) -> Self {
        let mut props = Map::new();
        props.insert("type".into(), ty.into());
        Self {
            props,
            problem: None,
        }
    }

    /// A datainfo built in code, or the datainfo of a type with one of its own (the enum
    /// and struct derives).
    pub fn from_info(info: Result<DataInfo, String>) -> Self {
        match info {
            Ok(info) => match info.to_json() {
                Json::Object(props) => Self {
                    props,
                    problem: None,
                },
                _ => Self::broken("the datainfo is not an object".into()),
            },
            Err(problem) => Self::broken(problem),
        }
    }

    /// The datainfo of `T`, which must have one of its own.
    pub fn of<T: SecopType>(type_name: &str) -> Self {
        Self::from_info(
            T::datainfo().ok_or_else(|| format!("`{type_name}` has no datainfo of its own")),
        )
    }

    fn broken(problem: String) -> Self {
        Self {
            props: Map::new(),
            problem: Some(problem),
        }
    }

    /// Sets a property.
    pub fn set(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.props.insert(key.to_owned(), value.into());
        self
    }

    /// Sets the `members` of an `array` or `tuple`.
    pub fn members(mut self, members: Vec<Di>) -> Self {
        let mut list = Vec::with_capacity(members.len());
        for member in members {
            match member.into_json() {
                Ok(json) => list.push(json),
                Err(problem) => self.problem = self.problem.or(Some(problem)),
            }
        }
        let value = if self.props.get("type").and_then(Json::as_str) == Some("array") {
            list.into_iter().next().unwrap_or(Json::Null)
        } else {
            Json::Array(list)
        };
        self.props.insert("members".into(), value);
        self
    }

    /// Fills `min` and `max` of an `int` that sets none from the range of the Rust type.
    pub fn int_bounds<T: SecopType>(mut self) -> Self {
        if self.props.get("type").and_then(Json::as_str) == Some("int")
            && let Some((min, max)) = T::int_range()
        {
            self.props.entry("min").or_insert(min.into());
            self.props.entry("max").or_insert(max.into());
        }
        self
    }

    /// A status tuple with the named SECoP status codes.
    pub fn status(codes: &[&str]) -> Self {
        let mut members = Map::new();
        for name in codes {
            match STATUS_CODES.iter().find(|(n, _)| n == name) {
                Some((n, code)) => {
                    members.insert((*n).to_owned(), (*code).into());
                }
                None => return Self::broken(format!("unknown status code `{name}`")),
            }
        }
        let mut code = Map::new();
        code.insert("type".into(), "enum".into());
        code.insert("members".into(), Json::Object(members));
        let mut text = Map::new();
        text.insert("type".into(), "string".into());
        Self::new("tuple").set(
            "members",
            Json::Array(vec![Json::Object(code), Json::Object(text)]),
        )
    }

    fn into_json(self) -> Result<Json, String> {
        match self.problem {
            Some(problem) => Err(problem),
            None => Ok(Json::Object(self.props)),
        }
    }

    /// Validates the datainfo.
    pub fn build(self) -> Result<DataInfo, String> {
        let json = self.into_json()?;
        DataInfo::from_json(&json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn double_with_unit() {
        let info = Di::new("double")
            .set("min", 0.0)
            .set("unit", "K")
            .set("fmtstr", "%.3f")
            .build()
            .unwrap();
        assert_eq!(
            info.to_json(),
            json!({"type": "double", "min": 0.0, "unit": "K", "fmtstr": "%.3f"})
        );
    }

    #[test]
    fn int_range_from_the_rust_type() {
        let info = Di::new("int").int_bounds::<u8>().build().unwrap();
        assert_eq!(info.to_json(), json!({"type": "int", "min": 0, "max": 255}));
        let info = Di::new("int")
            .set("max", 5)
            .int_bounds::<u8>()
            .build()
            .unwrap();
        assert_eq!(info.to_json(), json!({"type": "int", "min": 0, "max": 5}));
    }

    #[test]
    fn status_codes() {
        let info = Di::status(&["IDLE", "BUSY", "ERROR"]).build().unwrap();
        assert_eq!(
            info.to_json(),
            json!({"type": "tuple", "members": [
                {"type": "enum", "members": {"IDLE": 100, "BUSY": 300, "ERROR": 400}},
                {"type": "string"}]})
        );
    }

    #[test]
    fn array_and_tuple_members() {
        let info = Di::new("array")
            .members(vec![Di::new("double")])
            .set("maxlen", 10)
            .build()
            .unwrap();
        assert_eq!(
            info.to_json(),
            json!({"type": "array", "members": {"type": "double"}, "maxlen": 10})
        );
        let info = Di::new("tuple")
            .members(vec![Di::new("double"), Di::new("string")])
            .build()
            .unwrap();
        assert_eq!(
            info.to_json(),
            json!({"type": "tuple", "members": [{"type": "double"}, {"type": "string"}]})
        );
    }

    #[test]
    fn invalid_datainfo() {
        let err = Di::new("double")
            .set("min", 2.0)
            .set("max", 1.0)
            .build()
            .unwrap_err();
        assert!(err.contains("min"), "{err}");
    }
}
