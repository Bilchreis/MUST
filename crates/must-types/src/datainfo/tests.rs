// SPDX-License-Identifier: LGPL-3.0-or-later

use serde_json::json;

use super::*;

#[allow(clippy::needless_pass_by_value)]
fn parse(json: Json) -> Result<DataInfo> {
    DataInfo::from_json(&json)
}

#[allow(clippy::needless_pass_by_value)]
fn kind(json: Json) -> DataInfoErrorKind {
    parse(json).expect_err("should fail").kind
}

// --- 3.1 type dispatch

#[test]
fn known_type() {
    let DataInfo::Double(d) =
        parse(json!({"type": "double", "min": 0, "max": 100, "unit": "K"})).unwrap()
    else {
        panic!("not a double")
    };
    assert_eq!(
        (d.min(), d.max(), d.unit()),
        (Some(0.0), Some(100.0), Some("K"))
    );
}

#[test]
fn unknown_type() {
    assert_eq!(
        kind(json!({"type": "float"})),
        DataInfoErrorKind::UnknownType("float".into())
    );
}

#[test]
fn matrix() {
    let DataInfo::Matrix(m) = parse(
        json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}),
    )
    .unwrap() else {
        panic!("not a matrix")
    };
    assert_eq!(m.names(), ["x", "y"]);
    assert_eq!(m.maxlen(), [100, 100]);
    assert_eq!(m.dims(), 2);
    let t = m.elementtype();
    assert_eq!(
        (t.big_endian(), t.kind(), t.size()),
        (false, ElementKind::Float, 4)
    );
}

#[test]
fn not_an_object() {
    assert_eq!(kind(json!("double")), DataInfoErrorKind::NotAnObject);
    assert_eq!(kind(json!([])), DataInfoErrorKind::NotAnObject);
    assert_eq!(kind(json!({"min": 0})), DataInfoErrorKind::MissingType);
    assert_eq!(kind(json!({"type": 3})), DataInfoErrorKind::MissingType);
}

#[test]
fn missing_mandatory_properties() {
    assert_eq!(
        kind(json!({"type": "int", "min": 0})),
        DataInfoErrorKind::MissingProperty("max")
    );
    assert_eq!(
        kind(json!({"type": "blob"})),
        DataInfoErrorKind::MissingProperty("maxbytes")
    );
    assert_eq!(
        kind(json!({"type": "scaled", "min": 0, "max": 1})),
        DataInfoErrorKind::MissingProperty("scale")
    );
    for ty in ["enum", "tuple", "struct"] {
        assert_eq!(
            kind(json!({"type": ty})),
            DataInfoErrorKind::MissingProperty("members")
        );
    }
    assert_eq!(
        kind(json!({"type": "array", "maxlen": 3})),
        DataInfoErrorKind::MissingProperty("members")
    );
    assert_eq!(
        kind(json!({"type": "array", "members": {"type": "bool"}})),
        DataInfoErrorKind::MissingProperty("maxlen")
    );
    assert_eq!(
        kind(json!({"type": "matrix", "names": ["x"], "maxlen": [10]})),
        DataInfoErrorKind::MissingProperty("elementtype")
    );
    assert_eq!(
        kind(json!({"type": "matrix", "elementtype": "<f4", "maxlen": [10]})),
        DataInfoErrorKind::MissingProperty("names")
    );
    assert_eq!(
        kind(json!({"type": "matrix", "elementtype": "<f4", "names": ["x"]})),
        DataInfoErrorKind::MissingProperty("maxlen")
    );
    let e = parse(json!({"type": "matrix", "names": ["x"], "maxlen": [10]})).unwrap_err();
    assert!(e.to_string().contains("`elementtype`"), "{e}");
}

// --- matrix properties

fn wrong_property(json: Json) -> &'static str {
    match kind(json) {
        DataInfoErrorKind::WrongPropertyType { property, .. } => property,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn matrix_big_endian_unsigned() {
    let DataInfo::Matrix(m) =
        parse(json!({"type": "matrix", "elementtype": ">u2", "names": ["ch"], "maxlen": [4096]}))
            .unwrap()
    else {
        panic!()
    };
    let t = m.elementtype();
    assert_eq!(
        (t.big_endian(), t.kind(), t.size()),
        (true, ElementKind::UInt, 2)
    );
}

#[test]
fn matrix_element_types() {
    for t in [
        "<i1", ">i2", "<i4", ">i8", "<u1", "<u8", "<f2", ">f4", "<f8",
    ] {
        let info =
            parse(json!({"type": "matrix", "elementtype": t, "names": ["x"], "maxlen": [1]}));
        assert!(info.is_ok(), "{t}");
        assert_eq!(t.parse::<ElementType>().unwrap().to_string(), t);
    }
    for t in [
        json!("<f1"),
        json!("<x4"),
        json!("f4"),
        json!("<i3"),
        json!("=i4"),
        json!("<i16"),
        json!("<i0"),
        json!(4),
    ] {
        let e = parse(json!({"type": "matrix", "elementtype": t, "names": ["x"], "maxlen": [1]}))
            .unwrap_err();
        assert!(
            matches!(
                e.kind,
                DataInfoErrorKind::WrongPropertyType {
                    property: "elementtype",
                    ..
                }
            ),
            "{t}: {e:?}"
        );
        assert!(e.to_string().contains("`elementtype`"), "{e}");
    }
}

#[test]
fn matrix_length_count_differs_from_names() {
    let json =
        json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100]});
    assert_eq!(wrong_property(json.clone()), "maxlen");
    assert!(parse(json).unwrap_err().to_string().contains("`maxlen`"));
    for maxlen in [
        json!([0]),
        json!([-1]),
        json!([1.5]),
        json!(["1"]),
        json!(10),
    ] {
        assert_eq!(
            wrong_property(
                json!({"type": "matrix", "elementtype": "<f4", "names": ["x"], "maxlen": maxlen})
            ),
            "maxlen"
        );
    }
}

#[test]
fn matrix_names() {
    let names = |names: Json| {
        wrong_property(
            json!({"type": "matrix", "elementtype": "<f4", "names": names, "maxlen": [1, 1]}),
        )
    };
    assert_eq!(names(json!(["x", "x"])), "names");
    assert_eq!(names(json!([])), "names");
    assert_eq!(names(json!(["x", 1])), "names");
    assert_eq!(names(json!("x")), "names");
    let e = parse(
        json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "x"], "maxlen": [1, 1]}),
    )
    .unwrap_err();
    assert!(e.to_string().contains("`names`"), "{e}");
}

#[test]
fn matrix_compression() {
    let e = parse(
        json!({"type": "matrix", "elementtype": "<f4", "names": ["x"],
                         "maxlen": [1], "compression": "zlib"}),
    )
    .unwrap_err();
    assert_eq!(
        e.kind,
        DataInfoErrorKind::UnsupportedProperty("compression")
    );
    assert_eq!(e.to_string(), "property `compression` is not supported");
}

#[test]
fn matrix_round_trip() {
    let json = json!({"type": "matrix", "elementtype": ">i8", "names": ["x", "y", "z"],
                      "maxlen": [2, 3, 4], "_note": "custom"});
    let info = parse(json.clone()).unwrap();
    assert_eq!(info.to_json(), json);
    assert_eq!(info.to_string().parse::<DataInfo>().unwrap(), info);
    assert_eq!(info.type_name(), "matrix");

    let built = DataInfo::matrix(">i8".parse().unwrap(), [("x", 2), ("y", 3), ("z", 4)])
        .prop("_note", "custom")
        .build()
        .unwrap();
    assert_eq!(built, info);
    assert!(
        built.matches(
            &parse(json!({"type": "matrix", "elementtype": ">i8",
        "names": ["x", "y", "z"], "maxlen": [2, 3, 4]}))
            .unwrap()
        )
    );
}

#[test]
fn property_types() {
    assert!(matches!(
        kind(json!({"type": "int", "min": 0.5, "max": 10})),
        DataInfoErrorKind::WrongPropertyType {
            property: "min",
            ..
        }
    ));
    assert!(matches!(
        kind(json!({"type": "string", "maxchars": -1})),
        DataInfoErrorKind::WrongPropertyType {
            property: "maxchars",
            ..
        }
    ));
    assert!(matches!(
        kind(json!({"type": "double", "unit": 5})),
        DataInfoErrorKind::WrongPropertyType {
            property: "unit",
            ..
        }
    ));
    assert!(matches!(
        kind(json!({"type": "double", "fmtstr": 3})),
        DataInfoErrorKind::WrongPropertyType {
            property: "fmtstr",
            ..
        }
    ));
    // integral floats are accepted for integer properties
    assert!(parse(json!({"type": "int", "min": 0.0, "max": 10.0})).is_ok());
}

// --- 3.2 numeric types

#[test]
fn consistent_limits() {
    assert_eq!(
        kind(json!({"type": "double", "min": 10, "max": 0})),
        DataInfoErrorKind::InvertedLimits {
            lower: "min",
            upper: "max"
        }
    );
    assert!(parse(json!({"type": "int", "min": 5, "max": 5})).is_ok());
    assert!(parse(json!({"type": "scaled", "scale": 1, "min": 3, "max": 2})).is_err());
}

#[test]
fn scale_factor() {
    for scale in [json!(0), json!(-0.1)] {
        assert_eq!(
            kind(json!({"type": "scaled", "scale": scale, "min": 0, "max": 10})),
            DataInfoErrorKind::InvalidScale
        );
    }
}

#[test]
fn fmtstr_syntax() {
    for f in ["%.3f", "%.12g", "%.0e"] {
        assert!(parse(json!({"type": "double", "fmtstr": f})).is_ok(), "{f}");
    }
    for f in ["%5.3f", "%d", "%.3", "%.05f", "%.123f", ".3f"] {
        assert_eq!(
            kind(json!({"type": "double", "fmtstr": f})),
            DataInfoErrorKind::InvalidFmtstr(f.into()),
            "{f}"
        );
    }
}

#[test]
fn double_defaults() {
    let DataInfo::Double(d) = parse(json!({"type": "double"})).unwrap() else {
        panic!()
    };
    assert_eq!(d.fmtstr(), "%.6g");
    assert!((d.absolute_resolution()).abs() < f64::EPSILON);
    assert!((d.relative_resolution() - 1.2e-7).abs() < f64::EPSILON);
}

#[test]
fn scaled_defaults() {
    let DataInfo::Scaled(s) =
        parse(json!({"type": "scaled", "scale": 0.01, "min": 0, "max": 1000})).unwrap()
    else {
        panic!()
    };
    assert_eq!(s.fmtstr(), "%.2f");
    assert!((s.absolute_resolution() - 0.01).abs() < f64::EPSILON);
    assert!((s.relative_resolution() - 1.2e-7).abs() < f64::EPSILON);

    let fmt = |scale: f64| {
        let DataInfo::Scaled(s) = DataInfo::scaled(scale, 0, 1).build().unwrap() else {
            panic!()
        };
        s.fmtstr()
    };
    assert_eq!(fmt(0.1), "%.1f");
    assert_eq!(fmt(0.001), "%.3f");
    assert_eq!(fmt(0.5), "%.1f");
    assert_eq!(fmt(1.0), "%.0f");
    assert_eq!(fmt(25.0), "%.0f");
}

// --- 3.3 enum, string, blob

#[test]
fn enum_duplicate_value() {
    assert_eq!(
        kind(json!({"type": "enum", "members": {"on": 1, "active": 1}})),
        DataInfoErrorKind::DuplicateEnumValue {
            value: 1,
            first: "on".into(),
            second: "active".into()
        }
    );
}

#[test]
fn enum_lookup_both_directions() {
    let DataInfo::Enum(e) =
        parse(json!({"type": "enum", "members": {"IDLE": 100, "BUSY": 300}})).unwrap()
    else {
        panic!()
    };
    assert_eq!(e.value_of("BUSY"), Some(300));
    assert_eq!(e.name_of(100), Some("IDLE"));
    assert_eq!(e.name_of(200), None);
}

#[test]
fn enum_names() {
    // names need not be identifiers, but must be non-empty and unique ignoring case
    assert!(parse(json!({"type": "enum", "members": {"heater on": 1, "1st": 2}})).is_ok());
    assert_eq!(
        kind(json!({"type": "enum", "members": {"": 1}})),
        DataInfoErrorKind::EmptyMemberName
    );
    assert!(matches!(
        kind(json!({"type": "enum", "members": {"On": 1, "on": 2}})),
        DataInfoErrorKind::MemberNameClash(_)
    ));
    assert!(parse(json!({"type": "enum", "members": {}})).is_ok());
    assert!(parse(json!({"type": "enum", "members": {"a": 1.5}})).is_err());
}

#[test]
fn length_bounds() {
    assert!(parse(json!({"type": "string", "minchars": 5, "maxchars": 3})).is_err());
    assert!(parse(json!({"type": "blob", "minbytes": 5, "maxbytes": 3})).is_err());
    let DataInfo::String(s) = parse(json!({"type": "string"})).unwrap() else {
        panic!()
    };
    assert_eq!((s.minchars(), s.maxchars(), s.is_utf8()), (0, None, false));
}

#[test]
fn large_maxbytes_on_any_platform() {
    let big = u64::from(u32::MAX) + 1;
    let DataInfo::Blob(b) = parse(json!({"type": "blob", "maxbytes": big})).unwrap() else {
        panic!()
    };
    assert_eq!(b.maxbytes(), big);
}

// --- 3.4 structured types

#[test]
fn nested_datainfo() {
    let DataInfo::Array(a) = parse(
        json!({"type": "array", "maxlen": 10, "members": {"type": "int", "min": 0, "max": 9}}),
    )
    .unwrap() else {
        panic!()
    };
    assert!(matches!(a.members(), DataInfo::Int(_)));
    assert_eq!((a.minlen(), a.maxlen()), (0, 10));
}

#[test]
fn struct_member_names_clash() {
    let e = parse(
        json!({"type": "struct", "members": {"X": {"type": "double"}, "x": {"type": "double"}}}),
    )
    .unwrap_err();
    assert!(e.to_string().contains("`x`"), "{e}");
}

#[test]
fn nested_command() {
    let e = parse(json!({"type": "tuple", "members": [{"type": "command"}]})).unwrap_err();
    assert_eq!(e.kind, DataInfoErrorKind::NestedCommand);
    assert_eq!(e.path.to_string(), "[0]");
    assert!(parse(json!({"type": "array", "maxlen": 1, "members": {"type": "command"}})).is_err());
    assert!(parse(json!({"type": "command", "argument": {"type": "command"}})).is_err());
}

#[test]
fn nested_error_path() {
    let e = parse(json!({"type": "struct", "members": {"p": {"type": "tuple", "members": [{"type": "bool"}, {"type": "int"}]}}}))
        .unwrap_err();
    assert_eq!(e.kind, DataInfoErrorKind::MissingProperty("min"));
    assert_eq!(e.to_string(), "p[1]: missing mandatory property `min`");
}

#[test]
fn unknown_optional_member() {
    assert_eq!(
        kind(json!({"type": "struct", "members": {"x": {"type": "double"}}, "optional": ["y"]})),
        DataInfoErrorKind::UnknownOptionalMember("y".into())
    );
    let DataInfo::Struct(s) =
        parse(json!({"type": "struct", "members": {"x": {"type": "double"}}, "optional": ["x"]}))
            .unwrap()
    else {
        panic!()
    };
    assert!(s.is_optional("x"));
}

#[test]
fn empty_tuple_and_struct_accepted() {
    assert!(parse(json!({"type": "tuple", "members": []})).is_ok());
    assert!(parse(json!({"type": "struct", "members": {}})).is_ok());
}

#[test]
fn command_without_argument() {
    let DataInfo::Command(c) =
        parse(json!({"type": "command", "result": {"type": "bool"}})).unwrap()
    else {
        panic!()
    };
    assert!(c.argument().is_none());
    assert!(matches!(c.result(), Some(DataInfo::Bool(_))));
    let DataInfo::Command(c) = parse(json!({"type": "command", "argument": null})).unwrap() else {
        panic!()
    };
    assert!(c.argument().is_none());
}

// --- 3.5 serialization and builders

#[test]
fn custom_property_round_trip() {
    let info = parse(json!({"type": "bool", "_note": "x"})).unwrap();
    assert_eq!(info.to_json(), json!({"type": "bool", "_note": "x"}));
    // a property known for another type is preserved as unknown, too
    let info = parse(json!({"type": "bool", "unit": "K"})).unwrap();
    assert_eq!(info.extra().get("unit"), Some(&json!("K")));
}

#[test]
fn round_trip_keeps_order() {
    let info = parse(
        json!({"type": "struct", "members": {"y": {"type": "double"}, "x": {"type": "bool"}}}),
    )
    .unwrap();
    let text = info.to_string();
    assert!(
        text.find("\"y\"").unwrap() < text.find("\"x\"").unwrap(),
        "{text}"
    );
    assert_eq!(text.parse::<DataInfo>().unwrap(), info);
}

#[test]
fn no_defaults_added() {
    let info = parse(json!({"type": "string"})).unwrap();
    assert_eq!(info.to_json(), json!({"type": "string"}));
}

#[test]
fn built_in_code_same_error() {
    let built = DataInfo::int(10, 0).build().unwrap_err();
    let parsed = parse(json!({"type": "int", "min": 10, "max": 0})).unwrap_err();
    assert_eq!(built, parsed);
}

#[test]
fn builders() {
    let status = DataInfo::tuple(&[
        DataInfo::enumeration([("IDLE", 100), ("BUSY", 300)])
            .build()
            .unwrap(),
        DataInfo::string().build().unwrap(),
    ])
    .build()
    .unwrap();
    assert_eq!(
        status.to_json(),
        json!({"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100, "BUSY": 300}},
            {"type": "string"}
        ]})
    );
    let double = DataInfo::double().build().unwrap();
    let s = DataInfo::structure([("x", &double), ("t", &double)])
        .optional(["t"])
        .build()
        .unwrap();
    assert!(matches!(&s, DataInfo::Struct(i) if i.is_optional("t")));
    let cmd = DataInfo::command()
        .argument(&s)
        .result(&DataInfo::bool().build().unwrap())
        .build();
    assert!(cmd.is_ok());
}

#[test]
fn serde_integration() {
    let info: DataInfo = serde_json::from_str(r#"{"type": "int", "min": 0, "max": 3}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        r#"{"type":"int","min":0,"max":3}"#
    );
    assert!(serde_json::from_str::<DataInfo>(r#"{"type": "int"}"#).is_err());
}
