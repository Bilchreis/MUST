// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `datainfo` examples of the SECoP 2.0 specification (`protocol/specification/
//! datainfo.rst`, SampleEnvironment/SECoP `a6e90eb`).

use must_types::{DataInfo, DataInfoErrorKind, Mode, PathSegment};
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
        json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}),
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

/// The specification's matrix value example: a 2×3 matrix of `<f4` holding 1 to 6.
#[test]
fn spec_matrix_value_example() {
    let info = DataInfo::from_json(
        &json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}),
    )
    .unwrap();
    let json = json!({"len": [2, 3], "blob": "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA"});
    let value = info.decode(&json, Mode::Request).unwrap();
    let DataInfo::Matrix(matrix) = &info else {
        panic!("not a matrix")
    };
    let (len, data) = value.as_matrix().unwrap();
    assert_eq!(len, [2, 3]);
    assert_eq!(
        matrix.to_f64s(data).unwrap(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
    assert_eq!(info.encode(&value).unwrap(), json);
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
