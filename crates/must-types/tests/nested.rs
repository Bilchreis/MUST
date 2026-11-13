// SPDX-License-Identifier: LGPL-3.0-or-later

//! Deeply nested datainfos (tuples of arrays, structs of structs, arrays of structs, ragged
//! arrays with `minlen` != `maxlen`, ...), modelled on the test modules of secop-ophyd
//! (`frappy_modules/ophyd_secop_test_modules.py`, `tests/test_dtype.py`): they parse and
//! serialize unchanged, their values travel through JSON unchanged, and a value violating
//! a limit deep inside is rejected with the path to the offending element.

use must_types::{DataInfo, DataInfoErrorKind, ErrorClass, Mode, Value};
use serde_json::{Value as Json, json};

/// A nested datainfo with values that fit it.
struct Case {
    name: &'static str,
    datainfo: Json,
    values: Vec<Json>,
}

fn case(name: &'static str, datainfo: Json, values: Vec<Json>) -> Case {
    Case {
        name,
        datainfo,
        values,
    }
}

fn double() -> Json {
    json!({"type": "double"})
}

fn metres() -> Json {
    json!({"type": "double", "min": 0.0, "max": 100.0, "unit": "m"})
}

fn int(min: i64, max: i64) -> Json {
    json!({"type": "int", "min": min, "max": max})
}

fn string() -> Json {
    json!({"type": "string", "maxchars": 20})
}

#[allow(clippy::needless_pass_by_value)]
fn array(members: Json, minlen: u64, maxlen: u64) -> Json {
    json!({"type": "array", "members": members, "minlen": minlen, "maxlen": maxlen})
}

#[allow(clippy::needless_pass_by_value)]
fn tuple(members: Vec<Json>) -> Json {
    json!({"type": "tuple", "members": members})
}

#[allow(clippy::needless_pass_by_value)]
fn structure(members: Json) -> Json {
    json!({"type": "struct", "members": members})
}

fn point() -> Json {
    structure(json!({"x": metres(), "y": metres()}))
}

fn mode() -> Json {
    json!({"type": "enum", "members": {"mode_zero": 0, "mode_one": 1, "mode_max": 2}})
}

#[allow(clippy::too_many_lines, reason = "one entry per case")]
fn catalogue() -> Vec<Case> {
    vec![
        // --- ragged arrays: inner lengths differ, minlen != maxlen on every level
        case(
            "ragged 2d",
            array(array(double(), 0, 10), 1, 4),
            vec![
                json!([[1.0], [2.0, 3.0, 4.0], []]),
                json!([[]]),
                json!([[0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5]]),
            ],
        ),
        case(
            "ragged 3d",
            array(array(array(int(0, 100), 0, 5), 1, 3), 0, 3),
            vec![
                json!([[[1, 2], [3]], [[]], [[4], [5, 6, 7], [8, 9, 10, 11, 12]]]),
                json!([]),
            ],
        ),
        case(
            "fixed 3d (ophyd arr3d)",
            array(array(array(int(0, 100), 2, 2), 2, 2), 2, 2),
            vec![json!([[[1, 2], [3, 4]], [[5, 6], [7, 8]]])],
        ),
        // --- tuples of arrays
        case(
            "tuple of arrays",
            tuple(vec![array(double(), 0, 50), string()]),
            vec![json!([[1.0, 2.0, 3.0], "blah"]), json!([[], ""])],
        ),
        case(
            "tuple of ragged arrays",
            tuple(vec![
                array(array(int(0, 9), 1, 3), 0, 4),
                array(json!({"type": "bool"}), 2, 5),
            ]),
            vec![json!([[[1], [2, 3], [4, 5, 6]], [true, false]])],
        ),
        // --- tuples of structs
        case(
            "tuple of structs",
            tuple(vec![
                point(),
                structure(json!({"name": string(), "on": {"type": "bool"}})),
            ]),
            vec![json!([{"x": 1.5, "y": 2.5}, {"name": "probe", "on": true}])],
        ),
        // --- structs of structs
        case(
            "struct of structs (ophyd nested_struct)",
            structure(json!({
                "number": {"type": "double", "min": 0.0, "max": 100.0, "unit": "s"},
                "string": string(),
                "tupl": tuple(vec![metres(), metres(), metres()]),
                "pos_struct": structure(json!({
                    "x": metres(), "y": metres(), "z": metres(),
                    "col": string(), "enum": mode(),
                })),
            })),
            vec![json!({
                "number": 5.0, "string": "blue", "tupl": [1.0, 2.0, 3.0],
                "pos_struct": {"x": 1.0, "y": 2.0, "z": 3.0, "col": "red", "enum": 2},
            })],
        ),
        case(
            "struct of structs of structs",
            structure(json!({"a": structure(json!({"b": structure(json!({"c": int(0, 9)}))}))})),
            vec![json!({"a": {"b": {"c": 7}}})],
        ),
        // --- arrays of structs
        case(
            "array of structs",
            array(
                structure(
                    json!({"a": double(), "b": string(), "c": tuple(vec![double(), double()])}),
                ),
                0,
                50,
            ),
            vec![
                json!([{"a": 1.0, "b": "x", "c": [1.0, 2.0]}, {"a": 2.0, "b": "y", "c": [3.0, 4.0]}]),
                json!([]),
            ],
        ),
        case(
            "ragged array of arrays of structs",
            array(array(point(), 0, 3), 1, 3),
            vec![json!([[{"x": 1.0, "y": 1.0}], [], [{"x": 2.0, "y": 2.0}, {"x": 3.0, "y": 3.0}]])],
        ),
        // --- structs of tuples
        case(
            "struct of tuples",
            structure(json!({
                "limits": tuple(vec![metres(), metres()]),
                "status": tuple(vec![
                    json!({"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}),
                    json!({"type": "string"}),
                ]),
            })),
            vec![json!({"limits": [0.0, 100.0], "status": [300, "ramping"]})],
        ),
        // --- structs with arrays of structs
        case(
            "struct with array of structs",
            structure(json!({
                "name": string(),
                "points": array(
                    structure(json!({"x": metres(), "y": metres(), "tags": array(string(), 0, 3)})),
                    1,
                    5,
                ),
            })),
            vec![json!({"name": "path", "points": [
                {"x": 0.0, "y": 0.0, "tags": []},
                {"x": 1.0, "y": 2.0, "tags": ["start", "fast"]},
            ]})],
        ),
        // --- struct of arrays (ophyd Test_Struct_of_arrays, mass spectrometer)
        case(
            "struct of arrays",
            structure(json!({
                "ints": array(int(-100, 100), 5, 5),
                "strings": array(string(), 5, 5),
                "floats": array(double(), 5, 5),
                "spectrum": array(array(double(), 0, 100), 0, 3),
            })),
            vec![json!({
                "ints": [1, 2, 3, 4, 5],
                "strings": ["a", "b", "c", "d", "e"],
                "floats": [0.1, 0.2, 0.3, 0.4, 0.5],
                "spectrum": [[1.0, 2.0], [], [3.0]],
            })],
        ),
        // --- arrays of tuples of arrays (ophyd test_dtype)
        case(
            "array of tuples of arrays",
            array(tuple(vec![array(double(), 0, 50), string()]), 0, 100),
            vec![json!([[[1.0, 2.0, 3.0], "blah"], [[], "empty"]])],
        ),
        case(
            "array of arrays of tuples of arrays",
            array(
                array(tuple(vec![array(double(), 0, 50), string()]), 0, 10),
                0,
                10,
            ),
            vec![json!([
                [[[1.0], "a"], [[2.0, 3.0], "b"]],
                [],
                [[[4.0, 5.0, 6.0], "c"]]
            ])],
        ),
        case(
            "array of tuples with nested tuple and 2d array",
            array(
                tuple(vec![
                    double(),
                    string(),
                    tuple(vec![double(), string()]),
                    array(array(double(), 0, 4), 0, 4),
                ]),
                0,
                50,
            ),
            vec![json!([[1.0, "a", [2.0, "b"], [[1.0, 2.0], [3.0]]]])],
        ),
        // --- the less common leaf types deep inside
        case(
            "scaled, enum, blob deep inside",
            structure(json!({
                "readings": array(
                    tuple(vec![
                        json!({"type": "scaled", "scale": 0.01, "min": 0, "max": 100_000, "unit": "mbar"}),
                        mode(),
                        json!({"type": "blob", "minbytes": 1, "maxbytes": 8}),
                    ]),
                    0,
                    4,
                ),
                "modes": array(array(mode(), 0, 3), 0, 3),
            })),
            vec![json!({
                "readings": [[12345, 1, "U0VDb1A="], [0, 0, "AA=="]],
                "modes": [[0, 1], [], [2]],
            })],
        ),
    ]
}

/// The datainfo of `case`; panics with the case name if it does not parse.
fn datainfo(case: &Case) -> DataInfo {
    DataInfo::from_json(&case.datainfo)
        .unwrap_or_else(|e| panic!("{}: datainfo does not parse: {e}", case.name))
}

#[test]
fn nested_datainfos_parse_and_round_trip() {
    for case in catalogue() {
        let info = datainfo(&case);
        assert_eq!(info.to_json(), case.datainfo, "{}: to_json", case.name);
        assert_eq!(
            info.to_string().parse::<DataInfo>().unwrap(),
            info,
            "{}: text round trip",
            case.name
        );
        assert!(info.matches(&info), "{}: matches itself", case.name);
    }
}

#[test]
fn nested_values_round_trip() {
    for case in catalogue() {
        let info = datainfo(&case);
        for json in &case.values {
            for mode in [Mode::Request, Mode::Report] {
                let value = info
                    .decode(json, mode)
                    .unwrap_or_else(|e| panic!("{}: {json} rejected ({mode:?}): {e}", case.name));
                assert_eq!(
                    info.encode(&value).as_ref(),
                    Ok(json),
                    "{}: encode",
                    case.name
                );
                // values travel as text on the wire
                let text = info.encode(&value).unwrap().to_string();
                let reparsed = info.decode(&serde_json::from_str(&text).unwrap(), mode);
                assert_eq!(reparsed.as_ref(), Ok(&value), "{}: via text", case.name);
            }
        }
    }
}

/// Decodes `json` as a request, expecting an error of `class` at `path`. Returns the
/// message.
fn rejected(info: &Json, json: &Json, class: ErrorClass, path: &str) -> String {
    let info = DataInfo::from_json(info).unwrap();
    let e = info
        .decode(json, Mode::Request)
        .expect_err(&format!("{json} accepted"));
    assert_eq!((e.class, e.path.to_string().as_str()), (class, path), "{e}");
    e.message
}

#[test]
fn ragged_array_lengths_are_checked_on_every_level() {
    let ragged = array(array(double(), 1, 3), 2, 4);
    // the outer array
    rejected(&ragged, &json!([[1.0]]), ErrorClass::RangeError, "");
    rejected(
        &ragged,
        &json!([[1.0], [1.0], [1.0], [1.0], [1.0]]),
        ErrorClass::RangeError,
        "",
    );
    // an inner array, which may be shorter or longer than its neighbours but not leave
    // its own limits
    let message = rejected(&ragged, &json!([[1.0], []]), ErrorClass::RangeError, "[1]");
    assert_eq!(message, "0 elements, at least 1 required");
    let message = rejected(
        &ragged,
        &json!([[1.0, 2.0, 3.0], [1.0, 2.0, 3.0, 4.0]]),
        ErrorClass::RangeError,
        "[1]",
    );
    assert_eq!(message, "4 elements, at most 3 allowed");

    let ragged_3d = array(array(array(int(0, 100), 0, 2), 1, 2), 0, 3);
    rejected(
        &ragged_3d,
        &json!([[[1]], [[1, 2], [3, 4, 5]]]),
        ErrorClass::RangeError,
        "[1][1]",
    );
    rejected(
        &ragged_3d,
        &json!([[[1]], []]),
        ErrorClass::RangeError,
        "[1]",
    );
    rejected(
        &ragged_3d,
        &json!([[[1]], [[101]]]),
        ErrorClass::RangeError,
        "[1][0][0]",
    );
}

#[test]
fn errors_deep_inside_carry_their_path() {
    let path_info = structure(json!({
        "name": string(),
        "points": array(
            structure(json!({"x": metres(), "y": metres(), "tags": array(string(), 0, 2)})),
            1,
            5,
        ),
    }));
    let good = |x: f64, tags: Json| json!({"x": x, "y": 0.0, "tags": tags});
    rejected(
        &path_info,
        &json!({"name": "p", "points": [good(1.0, json!([])), good(101.0, json!([]))]}),
        ErrorClass::RangeError,
        "points[1].x",
    );
    rejected(
        &path_info,
        &json!({"name": "p", "points": [good(1.0, json!(["a", "b", "c"]))]}),
        ErrorClass::RangeError,
        "points[0].tags",
    );
    rejected(
        &path_info,
        &json!({"name": "p", "points": [good(1.0, json!(["this string is far too long"]))]}),
        ErrorClass::RangeError,
        "points[0].tags[0]",
    );
    rejected(
        &path_info,
        &json!({"name": "p", "points": [good(1.0, json!([1]))]}),
        ErrorClass::WrongType,
        "points[0].tags[0]",
    );
    let message = rejected(
        &path_info,
        &json!({"name": "p", "points": [{"x": 1.0, "tags": []}]}),
        ErrorClass::WrongType,
        "points[0]",
    );
    assert_eq!(message, "missing struct member `y`");
    rejected(
        &path_info,
        &json!({"name": "p", "points": [{"x": 1.0, "y": 1.0, "z": 1.0, "tags": []}]}),
        ErrorClass::WrongType,
        "points[0]",
    );

    let tuple_of_structs = tuple(vec![point(), array(point(), 0, 2)]);
    rejected(
        &tuple_of_structs,
        &json!([{"x": 1.0, "y": 1.0}, [{"x": 1.0, "y": -1.0}]]),
        ErrorClass::RangeError,
        "[1][0].y",
    );
    let message = rejected(
        &tuple_of_structs,
        &json!([{"x": 1.0, "y": 1.0}, [[1.0, 1.0]]]),
        ErrorClass::WrongType,
        "[1][0]",
    );
    assert_eq!(message, "expected an object, got an array");

    let array_of_tuples = array(tuple(vec![array(double(), 0, 50), string()]), 0, 10);
    let message = rejected(
        &array_of_tuples,
        &json!([[[1.0], "a"], [[2.0]]]),
        ErrorClass::WrongType,
        "[1]",
    );
    assert_eq!(message, "expected 2 tuple members, got 1");
    rejected(
        &array_of_tuples,
        &json!([[[1.0], "a"], [[2.0, "x"], "b"]]),
        ErrorClass::WrongType,
        "[1][0][1]",
    );

    let enums = structure(json!({"modes": array(array(mode(), 0, 3), 0, 3)}));
    rejected(
        &enums,
        &json!({"modes": [[0], [1, 7]]}),
        ErrorClass::RangeError,
        "modes[1][1]",
    );
}

/// A node may report values outside its declared limits, also deep inside: reports are
/// only checked for their shape.
#[test]
fn reports_are_not_checked_against_limits_deep_inside() {
    let info = DataInfo::from_json(&array(
        structure(json!({"x": metres(), "samples": array(int(0, 9), 1, 2)})),
        1,
        2,
    ))
    .unwrap();
    let out_of_limits = json!([{"x": 500.0, "samples": []}, {"x": -1.0, "samples": [10, 11, 12]},
                               {"x": 0.0, "samples": [0]}]);
    assert_eq!(
        info.decode(&out_of_limits, Mode::Request)
            .unwrap_err()
            .class,
        ErrorClass::RangeError
    );
    let value = info.decode(&out_of_limits, Mode::Report).unwrap();
    assert_eq!(info.encode(&value), Ok(out_of_limits));

    // but the shape must fit
    let missing = json!([{"x": 1.0}]);
    let e = info.decode(&missing, Mode::Report).unwrap_err();
    assert_eq!(
        (e.class, e.path.to_string()),
        (ErrorClass::WrongType, "[0]".into())
    );
}

#[test]
fn optional_members_of_nested_structs() {
    let info = DataInfo::from_json(&array(
        json!({"type": "struct", "members": {"x": double(), "t": double(), "note": string()},
               "optional": ["t", "note"]}),
        0,
        3,
    ))
    .unwrap();
    // a request may leave out optional members, also inside an array
    let value = info
        .decode(
            &json!([{"x": 1.0}, {"x": 2.0, "t": 3.0}, {"x": 3.0, "t": 4.0, "note": "n"}]),
            Mode::Request,
        )
        .unwrap();
    let Value::Array(items) = &value else {
        panic!("{value:?}")
    };
    let names = |v: &Value| match v {
        Value::Struct(m) => m.keys().cloned().collect::<Vec<_>>(),
        other => panic!("{other:?}"),
    };
    assert_eq!(names(&items[0]), ["x"]);
    assert_eq!(names(&items[1]), ["x", "t"]);
    // mandatory ones may not be left out
    let e = info
        .decode(&json!([{"x": 1.0}, {"t": 3.0}]), Mode::Request)
        .unwrap_err();
    assert_eq!(
        (e.class, e.path.to_string()),
        (ErrorClass::WrongType, "[1]".into())
    );
}

#[test]
fn invalid_nested_datainfos_name_the_place() {
    let cases = [
        (
            // minlen > maxlen of an inner array
            structure(json!({"a": array(array(double(), 5, 2), 0, 3)})),
            "a[]",
            DataInfoErrorKind::InvertedLimits {
                lower: "minlen",
                upper: "maxlen",
            },
        ),
        (
            json!({"type": "array", "members": tuple(vec![double(), json!({"type": "array", "members": double()})]), "maxlen": 3}),
            "[][1]",
            DataInfoErrorKind::MissingProperty("maxlen"),
        ),
        (
            tuple(vec![array(structure(json!({"p": {"type": "int"}})), 0, 3)]),
            "[0][].p",
            DataInfoErrorKind::MissingProperty("min"),
        ),
    ];
    for (json, path, kind) in cases {
        let e = DataInfo::from_json(&json).unwrap_err();
        assert_eq!((e.path.to_string().as_str(), &e.kind), (path, &kind), "{e}");
    }
}

/// `matches` (used for postfixed parameters like `target` vs. `value`) ignores custom
/// properties on every level, but not limits or member names.
#[test]
fn nested_datainfos_match_ignoring_custom_properties() {
    let info = |inner: Json| {
        DataInfo::from_json(&array(structure(json!({"x": metres(), "l": inner})), 0, 5)).unwrap()
    };
    let a = info(array(int(0, 9), 0, 3));
    let mut custom = array(int(0, 9), 0, 3);
    custom["_source"] = json!("sensor 2");
    custom["members"]["_raw"] = json!(true);
    assert!(a.matches(&info(custom)));
    assert!(!a.matches(&info(array(int(0, 9), 0, 4))));
    assert!(!a.matches(&info(array(int(0, 8), 0, 3))));
    assert!(!a.matches(&info(array(array(int(0, 9), 0, 3), 0, 3))));
}
