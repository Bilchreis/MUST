// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `datainfo` examples of the SECoP 2.0 specification (`protocol/specification/
//! datainfo.rst`, SampleEnvironment/SECoP `a6e90eb`), except `matrix`, which must does
//! not support yet.

use must_types::{DataInfo, DataInfoErrorKind, PathSegment};
use serde_json::{Value, json};

fn examples() -> Vec<Value> {
    vec![
        json!({"type": "double", "min": 0, "max": 100, "fmtstr": "%.3f"}),
        json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}),
        json!({"type": "int", "min": 0, "max": 100}),
        json!({"type": "bool"}),
        json!({"type": "enum", "members": {"IDLE": 100, "WARN": 200, "BUSY": 300, "ERROR": 400}}),
        json!({"type": "string", "maxchars": 80}),
        json!({"type": "blob", "minbytes": 1, "maxbytes": 64}),
        json!({"type": "array", "minlen": 3, "maxlen": 10,
               "members": {"type": "int", "min": 0, "max": 9}}),
        json!({"type": "tuple", "members": [{"type": "int", "min": 0, "max": 999},
                                            {"type": "string", "maxchars": 80}]}),
        json!({"type": "struct", "members": {"y": {"type": "double"},
               "x": {"type": "enum", "members": {"On": 1, "Off": 0}}}}),
        // the spec text has a stray `}` before "optional"
        json!({"type": "struct",
               "members": {"x": {"type": "double"}, "y": {"type": "double"}, "t": {"type": "double"}},
               "optional": ["t"]}),
        json!({"type": "command"}),
        // the spec's command examples with the mandatory int limits added, see below
        json!({"type": "command", "argument": {"type": "bool"},
               "result": {"type": "int", "min": 0, "max": 100}}),
        json!({"type": "command",
               "argument": {"type": "struct", "members": {"p": {"type": "double"},
                                                          "i": {"type": "double"},
                                                          "d": {"type": "double"}}},
               "result": {"type": "tuple", "members": [{"type": "int", "min": 0, "max": 100},
                                                       {"type": "string"}]}}),
    ]
}

#[test]
fn spec_examples_parse_and_round_trip() {
    for example in examples() {
        let info = DataInfo::from_json(&example)
            .unwrap_or_else(|e| panic!("{example} does not parse: {e}"));
        let reparsed = DataInfo::from_json(&info.to_json()).unwrap();
        assert_eq!(reparsed, info, "{example} does not round-trip");
        // only given properties are written, in a canonical order
        assert_eq!(
            keys(&info.to_json()),
            keys(&example),
            "{example} gained or lost properties"
        );
    }
}

/// All object keys of a JSON value, recursively, sorted.
fn keys(json: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    match json {
        Value::Object(map) => {
            for (k, v) in map {
                keys.push(k.clone());
                keys.extend(self::keys(v).into_iter().map(|sub| format!("{k}.{sub}")));
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                keys.extend(self::keys(v).into_iter().map(|sub| format!("[{i}].{sub}")));
            }
        }
        _ => {}
    }
    keys.sort();
    keys
}

/// The spec's two `command` examples use `{"type": "int"}` without `min`/`max`, although
/// the same chapter (and `schema/datatypes.yaml`) makes both mandatory for `int`. must
/// follows the mandatory rule, so the examples as printed are rejected.
#[test]
fn spec_command_examples_lack_mandatory_int_limits() {
    let first = json!({"type": "command", "argument": {"type": "bool"}, "result": {"type": "int"}});
    let e = DataInfo::from_json(&first).unwrap_err();
    assert_eq!(e.kind, DataInfoErrorKind::MissingProperty("min"));
    assert_eq!(e.path.segments(), [PathSegment::Result]);

    let second = json!({"type": "command",
        "argument": {"type": "struct", "members": {"p": {"type": "double"}}},
        "result": {"type": "tuple", "members": [{"type": "int"}, {"type": "string"}]}});
    let e = DataInfo::from_json(&second).unwrap_err();
    assert_eq!(e.kind, DataInfoErrorKind::MissingProperty("min"));
    assert_eq!(e.path.to_string(), "result[0]");
}
