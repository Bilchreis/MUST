// SPDX-License-Identifier: LGPL-3.0-or-later

//! Data reports and error reports, the JSON payloads of SECoP messages.
//!
//! A data report is `[<value>, <qualifiers>]`, e.g. `[15.0, {"t": 1723712392.1}]`; an
//! error report is `[<class>, <text>, <extra>]`, e.g.
//! `["HardwareError", "heater is broken", {"t": 1723812391.1}]`.

use serde_json::{Map, Value as Json, json};

use crate::error::SecopError;

/// Qualifiers of a value: timestamp `t` and uncertainty `e`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Qualifiers {
    /// Time the value was obtained, as UNIX time in seconds.
    pub t: Option<f64>,
    /// Uncertainty of the value, in the value's unit.
    pub e: Option<Json>,
}

impl Qualifiers {
    /// Qualifiers with only a timestamp.
    pub fn at(t: f64) -> Self {
        Self {
            t: Some(t),
            e: None,
        }
    }

    fn to_json(&self) -> Map<String, Json> {
        let mut map = Map::new();
        if let Some(t) = self.t.and_then(serde_json::Number::from_f64) {
            map.insert("t".into(), Json::Number(t));
        }
        if let Some(e) = &self.e {
            map.insert("e".into(), e.clone());
        }
        map
    }
}

/// Builds a data report from an encoded value and its qualifiers.
pub fn data_report(value: Json, qualifiers: &Qualifiers) -> Json {
    Json::Array(vec![value, Json::Object(qualifiers.to_json())])
}

/// Builds an error report; `t` is added to the error's extra information when given.
pub fn error_report(error: &SecopError, t: Option<f64>) -> Json {
    let mut extra = error.extra.clone();
    if let Some(t) = t.and_then(serde_json::Number::from_f64) {
        extra.insert("t".into(), Json::Number(t));
    }
    json!([error.class.as_str(), error.message, extra])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorClass;

    #[test]
    fn data_report_shape() {
        assert_eq!(
            data_report(json!(15.0), &Qualifiers::at(1.5)),
            json!([15.0, {"t": 1.5}])
        );
        let q = Qualifiers {
            t: Some(2.0),
            e: Some(json!(0.01)),
        };
        assert_eq!(
            data_report(json!(295.13), &q),
            json!([295.13, {"t": 2.0, "e": 0.01}])
        );
        assert_eq!(
            data_report(Json::Null, &Qualifiers::default()),
            json!([null, {}])
        );
    }

    #[test]
    fn error_report_shape() {
        let e = SecopError::new(ErrorClass::HardwareError, "heater is broken");
        assert_eq!(
            error_report(&e, Some(3.0)),
            json!(["HardwareError", "heater is broken", {"t": 3.0}])
        );
        let e = e.with_extra("stack", json!([]));
        assert_eq!(
            error_report(&e, None),
            json!(["HardwareError", "heater is broken", {"stack": []}])
        );
    }

    #[test]
    fn error_class_names_round_trip() {
        assert_eq!(ErrorClass::ALL.len(), 21);
        for class in ErrorClass::ALL {
            assert_eq!(class.as_str().parse::<ErrorClass>(), Ok(*class));
        }
        assert_eq!(ErrorClass::BadJSON.to_string(), "BadJSON");
        assert!(
            "CommandFailed".parse::<ErrorClass>().is_err(),
            "SECoP 1.0 class, gone in 2.0"
        );
    }

    #[test]
    fn value_error_becomes_secop_error() {
        let info: crate::DataInfo =
            r#"{"type": "array", "maxlen": 3, "members": {"type": "int", "min": 0, "max": 9}}"#
                .parse()
                .unwrap();
        let e: SecopError = info
            .decode(&json!([1, 99]), crate::Mode::Request)
            .unwrap_err()
            .into();
        assert_eq!(e.class, ErrorClass::RangeError);
        assert!(e.message.starts_with("[1]: "), "{e}");
    }
}
