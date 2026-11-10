// SPDX-License-Identifier: LGPL-3.0-or-later

//! Building datainfos in code.

use serde_json::{Map, Value as Json};

use super::{DataInfo, ElementType, Result};

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
    /// Starts a `matrix` datainfo with dimensions given as name and maximum length,
    /// first (fastest varying) dimension first.
    pub fn matrix<'a>(
        elementtype: ElementType,
        dims: impl IntoIterator<Item = (&'a str, u64)>,
    ) -> DataInfoBuilder {
        let (names, maxlen): (Vec<Json>, Vec<Json>) = dims
            .into_iter()
            .map(|(name, maxlen)| (Json::from(name), Json::from(maxlen)))
            .unzip();
        DataInfoBuilder::new("matrix")
            .prop("elementtype", elementtype.to_string())
            .prop("names", names)
            .prop("maxlen", maxlen)
    }
    /// Starts a `command` datainfo.
    pub fn command() -> DataInfoBuilder {
        DataInfoBuilder::new("command")
    }
}
