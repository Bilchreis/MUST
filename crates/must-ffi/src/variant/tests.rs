// SPDX-License-Identifier: LGPL-3.0-or-later

use super::*;

#[allow(clippy::needless_pass_by_value)]
fn describe(j: Json) -> Node {
    Node::from_description(&j, false).unwrap()
}

#[test]
fn descriptions() {
    let d = describe(json!({"type": "DOUBLE", "min": 0, "max": 10, "unit": "K"}));
    assert_eq!(d.code(), vt::DOUBLE);
    assert_eq!(d.extra.get("unit"), Some(&json!("K")));
    assert_eq!(
        d.export_type(),
        Some(json!({"type": "double", "min": 0.0, "max": 10.0, "unit": "K"}))
    );

    // SECoP 1.0: int without limits
    let i = describe(json!({"type": "int"}));
    assert_eq!(i.export_type(), Some(json!({"type": "int"})));
    assert_eq!(
        i.to_datainfo().unwrap().to_json(),
        json!({"type": "int", "min": -16_777_216, "max": 16_777_216})
    );

    let a = describe(json!({"type": "array", "members": {"type": "int"}, "maxlen": 5}));
    assert_eq!(
        (a.code(), a.count()),
        (vt::ARRAY_INTEGER, 0),
        "starts at minlen"
    );
    let s =
        describe(json!({"type": "array", "members": {"type": "string"}, "minlen": 2, "maxlen": 5}));
    assert_eq!((s.code(), s.count()), (vt::ARRAY, 2));

    assert!(Node::from_description(&json!({"type": "nope"}), false).is_none());
    assert!(Node::from_description(&json!({"type": "command"}), false).is_none());
    assert!(Node::from_description(&json!({"type": "command"}), true).is_some());
    assert!(
        Node::from_description(&json!({"type": "double", "min": 5, "max": 1}), false).is_none()
    );
}

#[test]
fn inferred_types() {
    let code = |j: Json| Node::from_json(&j).map(|n| n.code());
    assert_eq!(code(json!(true)), Some(vt::BOOL));
    assert_eq!(code(json!(1.5)), Some(vt::DOUBLE));
    assert_eq!(code(json!(3)), Some(vt::INTEGER));
    assert_eq!(code(json!("x")), Some(vt::STRING));
    assert_eq!(code(json!({"a": 1})), Some(vt::STRUCT));
    assert_eq!(code(json!([1.0, 2.5])), Some(vt::ARRAY_DOUBLE));
    assert_eq!(code(json!([1, 2])), Some(vt::ARRAY_INTEGER));
    assert_eq!(code(json!([true])), Some(vt::ARRAY_BOOL));
    assert_eq!(code(json!(["a", "b"])), Some(vt::ARRAY));
    assert_eq!(code(json!([1, "b"])), Some(vt::TUPLE));
    assert_eq!(code(json!(null)), Some(vt::NONE));
    assert_eq!(code(json!([])), None);
}

#[test]
fn import_with_hint_type() {
    let mut s = describe(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
    assert!(s.import(&json!(1255), true));
    assert_eq!(s.export_value(), json!(1255));
    assert!(!s.import(&json!(2501), true));
    let mut b = describe(json!({"type": "blob", "maxbytes": 8}));
    assert!(b.import(&json!("U0VDb1A="), true));
    assert_eq!(b.count(), 5);
    assert_eq!(b.export_value(), json!("U0VDb1A="));
}

#[test]
fn walk_positions() {
    let n = Node::from_json(&json!({"a": 1.0, "b": "x"})).unwrap();
    assert_eq!(n.locate(0), Some((vec![], None)));
    assert_eq!(n.locate(1), Some((vec![0], Some("a".into()))));
    assert_eq!(n.locate(2), Some((vec![1], Some("b".into()))));
    assert_eq!(n.locate(3), None);
    assert_eq!(n.positions(), 3);

    let nested = Node::from_json(&json!({"p": {"x": 1, "y": [1, "z"]}, "q": true})).unwrap();
    // 0 root, 1 p (container: no name), 2 x, 3 y (tuple), 4 1, 5 "z", 6 q
    assert_eq!(nested.locate(1), Some((vec![0], None)));
    assert_eq!(nested.locate(2), Some((vec![0, 0], Some("x".into()))));
    assert_eq!(nested.locate(5), Some((vec![0, 1, 1], None)));
    assert_eq!(nested.locate(6), Some((vec![1], Some("q".into()))));
    assert_eq!(nested.locate(7), None);
}

#[test]
fn comparison() {
    let a = Node::from_json(&json!(1.5)).unwrap();
    let s = Node::from_json(&json!("x")).unwrap();
    assert_eq!(a.compare(&s), vc::DIFF_TYPE);
    assert_eq!(a.compare(&a.clone()), vc::EQUAL);
    let mut b = a.clone();
    b.import(&json!(2.0), false);
    assert_eq!(a.compare(&b), vc::DIFF_VALUE);
    let limited = describe(json!({"type": "double", "min": 0, "max": 10}));
    let mut limited = limited;
    limited.import(&json!(1.5), true);
    assert_eq!(a.compare(&limited), vc::SIMILAR_VALUE);
}

#[test]
fn handle_table() {
    let h = new_handle(Node::from_json(&json!(1)).unwrap());
    assert_eq!(with_handle(h, |n| n.code()), Some(vt::INTEGER));
    assert!(destroy_handle(h));
    assert!(!destroy_handle(h));
    assert_eq!(with_handle(h, |n| n.code()), None);
}

#[test]
fn destroyed_handles_are_not_reused() {
    let h = new_handle(Node::from_json(&json!(1)).unwrap());
    assert!(destroy_handle(h));
    // new values, even when their memory takes the place of the destroyed one
    let others: Vec<_> = (0..1000)
        .map(|_| new_handle(Node::from_json(&json!(2.5)).unwrap()))
        .collect();
    assert!(!others.contains(&h));
    assert_eq!(with_handle(h, |n| n.code()), None);
    for o in others {
        assert!(destroy_handle(o));
    }
}

// --- matrix

const IMAGE: &str = r#"{"type":"matrix","elementtype":"<f4","names":["x","y"],"maxlen":[100,100]}"#;
const SPEC_BLOB: &str = "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA";

fn image() -> Node {
    describe(serde_json::from_str(IMAGE).unwrap())
}

fn image_info() -> DataInfo {
    IMAGE.parse().unwrap()
}

#[test]
fn matrix_from_datainfo() {
    let m = image();
    assert_eq!((m.code(), m.count()), (vt::STRUCT, 2));
    assert_eq!(m.locate(1), Some((vec![0], Some("len".into()))));
    assert_eq!(m.locate(2), Some((vec![1], Some("blob".into()))));
    assert_eq!(m.locate(3), None);
    let len = m.at(&[0]).unwrap();
    assert_eq!((len.code(), len.count()), (vt::ARRAY_INTEGER, 2));
    assert_eq!(m.at(&[1]).unwrap().code(), vt::BLOB);
    assert_eq!(m.export_value(), json!({"len": [0, 0], "blob": ""}));
    // the type is still the matrix, also case-insensitively
    assert_eq!(m.to_datainfo(), Some(image_info()));
    assert_eq!(m.export_type(), Some(image_info().to_json()));
    let upper = describe(json!({"type": "MATRIX", "elementtype": "<f4",
        "names": ["x", "y"], "maxlen": [100, 100]}));
    assert_eq!(upper, m);
    // must-types validates the description
    assert!(
        Node::from_description(
            &json!({"type": "matrix", "elementtype": "<f1", "names": ["x"], "maxlen": [1]}),
            false
        )
        .is_none()
    );
    assert!(
        Node::from_description(
            &json!({"type": "array", "maxlen": 2,
            "members": {"type": "matrix", "elementtype": "<f4", "names": ["x"], "maxlen": [1]}}),
            false
        )
        .is_some()
    );
}

#[test]
fn matrix_from_transport_json_with_hint() {
    let mut m = image();
    let json = json!({"len": [2, 3], "blob": SPEC_BLOB});
    assert!(m.import(&json, true));
    let Kind::Bytes { bytes, .. } = &m.at(&[1]).unwrap().kind else {
        panic!()
    };
    let floats: Vec<f32> = bytes
        .chunks(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(floats, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(m.export_value(), json);
    assert_eq!(m.to_datainfo(), Some(image_info()));
    // the number of lengths is the number of dimensions
    assert!(!image().import(&json!({"len": [6], "blob": SPEC_BLOB}), true));
}

#[test]
fn matrix_datainfo_round_trip() {
    let info = image().to_datainfo().unwrap();
    let again = describe(info.to_json());
    assert_eq!(again.to_datainfo(), Some(info));
    // a node that lost the matrix shape is a plain struct again
    let mut broken = image();
    let Kind::Struct { items, .. } = &mut broken.kind else {
        panic!()
    };
    items.pop();
    assert_eq!(
        broken.export_type().unwrap()["type"],
        json!("struct"),
        "{broken:?}"
    );
}

#[test]
fn matrix_direct_and_json_paths_agree() {
    let info = image_info();
    let mut m = image();
    assert!(m.import(&json!({"len": [2, 3], "blob": SPEC_BLOB}), false));
    let via_json = info.decode(&m.export_value(), Mode::Report).unwrap();
    assert_eq!(m.to_value_direct(&info), Ok(via_json.clone()));
    assert_eq!(m.clone().into_value_direct(&info), Ok(via_json.clone()));

    // back into a handle: the same node either way
    let direct = image().with_value_direct(via_json.clone(), &info).unwrap();
    let mut json = image();
    assert!(json.import(&info.encode(&via_json).unwrap(), false));
    assert_eq!(direct, json);
    assert_eq!(direct, m);

    // a struct built without a hint has a string `blob` and goes through JSON
    let plain = Node::from_json(&json!({"len": [2, 3], "blob": SPEC_BLOB})).unwrap();
    assert!(plain.matrix_parts().is_none());
    assert_eq!(plain.to_value_direct(&info), Ok(via_json));
}

#[test]
fn matrix_errors_agree() {
    let info = image_info();
    for value in [
        json!({"len": [2, 3], "blob": "AACAPw=="}),
        json!({"len": [-1, 3], "blob": ""}),
        json!({"len": [4_294_967_296_u64, 1], "blob": ""}),
    ] {
        let mut m = image();
        assert!(m.import(&value, false), "{value}");
        let via_json = info.decode(&m.export_value(), Mode::Report).unwrap_err();
        assert_eq!(m.to_value_direct(&info).unwrap_err(), via_json, "{value}");
        assert_eq!(m.into_value_direct(&info).unwrap_err(), via_json, "{value}");
    }
    // a matrix with too few lengths
    let mut m = image();
    let Kind::Struct { items, .. } = &mut m.kind else {
        panic!()
    };
    let Kind::ScalarArray { values, .. } = &mut items[0].1.kind else {
        panic!()
    };
    values.pop();
    let via_json = info.decode(&m.export_value(), Mode::Report).unwrap_err();
    assert_eq!(m.to_value_direct(&info).unwrap_err(), via_json);
}

#[test]
fn struct_direct_and_json_paths_agree() {
    let info: DataInfo =
        r#"{"type":"struct","members":{"x":{"type":"double"},"n":{"type":"blob","maxbytes":4}}}"#
            .parse()
            .unwrap();
    let proto = describe(info.to_json());
    let value = info
        .decode(&json!({"x": 1.5, "n": "AQI="}), Mode::Report)
        .unwrap();
    let node = proto.with_value_direct(value.clone(), &info).unwrap();
    let mut json = proto.clone();
    assert!(json.import(&info.encode(&value).unwrap(), false));
    // (NaN limits make `==` useless here)
    assert_eq!(node.compare(&json), vc::EQUAL);
    assert_eq!(node.to_value_direct(&info), Ok(value.clone()));
    assert_eq!(node.into_value_direct(&info), Ok(value));
    let bad = Node::from_json(&json!({"x": "a"})).unwrap();
    assert_eq!(
        bad.to_value_direct(&info).unwrap_err(),
        info.decode(&bad.export_value(), Mode::Report).unwrap_err()
    );
}

#[test]
fn take_handle_moves_the_node() {
    let h = new_handle(image());
    assert_eq!(take_handle(h), Some(image()));
    assert_eq!(take_handle(h), None);
}
