// SPDX-License-Identifier: LGPL-3.0-or-later

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;

use super::*;
use crate::datainfo::{DataInfo, MatrixInfo};

#[allow(clippy::needless_pass_by_value)]
fn info(json: Json) -> DataInfo {
    DataInfo::from_json(&json).unwrap()
}

#[allow(clippy::needless_pass_by_value)]
fn class_of(result: Result<Value>) -> ErrorClass {
    result.expect_err("should fail").class
}

// --- 4.1 errors and paths

#[test]
fn error_display() {
    let e = ValueError::range("too big").within(PathSegment::Member("x".into()));
    let e = e.within(PathSegment::Index(2));
    assert_eq!(e.to_string(), "RangeError: [2].x: too big");
    assert_eq!(ValueError::wrong_type("bad").to_string(), "WrongType: bad");
    assert_eq!(ErrorClass::WrongType.as_str(), "WrongType");
}

// --- 4.2 numeric, enum, bool

#[test]
fn report_mode_ignores_range() {
    let d = info(json!({"type": "double", "min": 0, "max": 100}));
    assert_eq!(
        d.decode(&json!(150.0), Mode::Report),
        Ok(Value::Double(150.0))
    );
    assert_eq!(
        class_of(d.decode(&json!(150.0), Mode::Request)),
        ErrorClass::RangeError
    );
}

#[test]
fn wrong_json_type() {
    let d = info(json!({"type": "double"}));
    assert_eq!(
        class_of(d.decode(&json!("hot"), Mode::Request)),
        ErrorClass::WrongType
    );
}

#[test]
fn numeric_values() {
    let d = info(json!({"type": "double"}));
    assert_eq!(d.decode(&json!(5), Mode::Request), Ok(Value::Double(5.0)));

    let i = info(json!({"type": "int", "min": 0, "max": 10}));
    assert_eq!(
        class_of(i.decode(&json!(2.5), Mode::Request)),
        ErrorClass::WrongType
    );
    assert_eq!(i.decode(&json!(3.0), Mode::Request), Ok(Value::Int(3)));
    assert_eq!(i.decode(&json!(10), Mode::Request), Ok(Value::Int(10)));
    assert_eq!(i.decode(&json!(0), Mode::Request), Ok(Value::Int(0)));
    assert_eq!(
        class_of(i.decode(&json!(11), Mode::Request)),
        ErrorClass::RangeError
    );
    assert_eq!(
        class_of(i.decode(&json!(1e30), Mode::Report)),
        ErrorClass::RangeError
    );
    assert_eq!(
        class_of(i.decode(&json!("3"), Mode::Report)),
        ErrorClass::WrongType
    );
}

#[test]
fn scaled_conversion() {
    let s = info(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
    assert_eq!(
        s.decode(&json!(1255), Mode::Request),
        Ok(Value::Scaled(1255))
    );
    let DataInfo::Scaled(si) = &s else { panic!() };
    assert!((si.to_physical(1255) - 125.5).abs() < 1e-9);
    assert_eq!(si.from_physical(12.34), Some(123));
    assert_eq!(si.from_physical(f64::INFINITY), None);
    assert_eq!(
        class_of(s.decode(&json!(2501), Mode::Request)),
        ErrorClass::RangeError
    );
}

#[test]
fn enum_values() {
    let e = info(json!({"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}));
    assert_eq!(e.decode(&json!(300), Mode::Report), Ok(Value::Enum(300)));
    assert_eq!(
        class_of(e.decode(&json!(250), Mode::Report)),
        ErrorClass::RangeError
    );
    assert_eq!(
        class_of(e.decode(&json!(250), Mode::Request)),
        ErrorClass::RangeError
    );
    assert_eq!(
        class_of(e.decode(&json!("IDLE"), Mode::Request)),
        ErrorClass::WrongType
    );
}

#[test]
fn bool_values() {
    let b = info(json!({"type": "bool"}));
    assert_eq!(b.decode(&json!(true), Mode::Request), Ok(Value::Bool(true)));
    assert_eq!(
        class_of(b.decode(&json!(1), Mode::Request)),
        ErrorClass::WrongType
    );
}

// --- 4.3 dynamic limits

#[test]
fn dynamic_limits() {
    let d = info(json!({"type": "double", "min": 0, "max": 300}));
    let limits = DynLimits {
        min: Some(10.0),
        max: Some(200.0),
    };

    let e = d.decode_request(&json!(250.0), Some(&limits)).unwrap_err();
    assert_eq!(e.class, ErrorClass::RangeError);
    assert!(e.message.contains("dynamic upper limit 200"), "{e}");

    assert_eq!(
        d.decode_request(&json!(150.0), Some(&limits)),
        Ok(Value::Double(150.0))
    );

    let upper_only = DynLimits {
        min: None,
        max: Some(200.0),
    };
    let e = d
        .decode_request(&json!(-5.0), Some(&upper_only))
        .unwrap_err();
    assert!(
        e.message.contains("lower limit 0") && !e.message.contains("dynamic"),
        "{e}"
    );

    assert_eq!(
        d.decode_with(&json!(250.0), Mode::Report, Some(&limits)),
        Ok(Value::Double(250.0))
    );

    let s = info(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 2500}));
    let max_1000 = DynLimits {
        min: None,
        max: Some(1000.0),
    };
    assert_eq!(
        class_of(s.decode_request(&json!(1500), Some(&max_1000))),
        ErrorClass::RangeError
    );

    let i = info(json!({"type": "int", "min": 0, "max": 100}));
    let lim = DynLimits::from_limits(&Value::Tuple(vec![Value::Int(5), Value::Int(50)])).unwrap();
    assert_eq!(
        class_of(i.decode_request(&json!(4), Some(&lim))),
        ErrorClass::RangeError
    );
    assert_eq!(i.decode_request(&json!(5), Some(&lim)), Ok(Value::Int(5)));
}

// --- 4.4 string and blob

#[test]
fn string_values() {
    let utf8 = info(json!({"type": "string", "maxchars": 3, "isUTF8": true}));
    assert_eq!(
        utf8.decode(&json!("äöü"), Mode::Request),
        Ok(Value::String("äöü".into()))
    );
    assert_eq!(
        class_of(utf8.decode(&json!("äöüß"), Mode::Request)),
        ErrorClass::RangeError
    );

    let ascii = info(json!({"type": "string"}));
    assert_eq!(
        class_of(ascii.decode(&json!("ä"), Mode::Request)),
        ErrorClass::RangeError
    );
    assert!(ascii.decode(&json!("ä"), Mode::Report).is_ok());

    let min = info(json!({"type": "string", "minchars": 2}));
    assert_eq!(
        class_of(min.decode(&json!("a"), Mode::Request)),
        ErrorClass::RangeError
    );
}

#[test]
fn blob_values() {
    let b = info(json!({"type": "blob", "maxbytes": 8}));
    assert_eq!(
        b.decode(&json!("U0VDb1A="), Mode::Request),
        Ok(Value::Blob(b"SECoP".to_vec()))
    );
    assert_eq!(
        b.decode(&json!("AA=="), Mode::Request),
        Ok(Value::Blob(vec![0]))
    );
    assert_eq!(
        class_of(b.decode(&json!("not base64!"), Mode::Request)),
        ErrorClass::WrongType
    );
    assert_eq!(
        class_of(b.decode(&json!("U0VDb1A"), Mode::Request)),
        ErrorClass::WrongType
    );
    let small = info(json!({"type": "blob", "maxbytes": 4}));
    assert_eq!(
        class_of(small.decode(&json!("U0VDb1A="), Mode::Request)),
        ErrorClass::RangeError
    );
    assert_eq!(
        b.encode(&Value::Blob(b"SECoP".to_vec())),
        Ok(json!("U0VDb1A="))
    );
}

// --- 4.5 array, tuple, struct

#[test]
fn nested_failure_path() {
    let a = info(
        json!({"type": "array", "maxlen": 10, "members": {"type": "int", "min": 0, "max": 9}}),
    );
    let e = a.decode(&json!([1, 2, 99]), Mode::Request).unwrap_err();
    assert_eq!(e.class, ErrorClass::RangeError);
    assert_eq!(e.path.segments(), [PathSegment::Index(2)]);
}

#[test]
fn array_and_tuple_values() {
    let a = info(json!({"type": "array", "maxlen": 3, "members": {"type": "bool"}}));
    assert_eq!(
        class_of(a.decode(&json!([true, true, true, true]), Mode::Request)),
        ErrorClass::RangeError
    );
    assert!(
        a.decode(&json!([true, true, true, true]), Mode::Report)
            .is_ok()
    );

    let t = info(
        json!({"type": "tuple", "members": [{"type": "int", "min": 0, "max": 999}, {"type": "string"}]}),
    );
    assert_eq!(
        class_of(t.decode(&json!([300]), Mode::Request)),
        ErrorClass::WrongType
    );
    assert_eq!(
        t.decode(&json!([300, "accelerating"]), Mode::Report),
        Ok(Value::Tuple(vec![
            Value::Int(300),
            Value::String("accelerating".into())
        ]))
    );
}

#[test]
fn struct_values() {
    let s = info(json!({"type": "struct",
        "members": {"x": {"type": "double"}, "y": {"type": "double"}, "t": {"type": "double"}},
        "optional": ["t"]}));
    let Value::Struct(v) = s
        .decode(&json!({"x": 1.0, "y": 2.0}), Mode::Request)
        .unwrap()
    else {
        panic!()
    };
    assert!(!v.contains_key("t"));

    let e = s
        .decode(&json!({"x": 1.0, "y": 2.0}), Mode::Report)
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert!(e.message.contains("`t`"), "{e}");

    let xy = info(
        json!({"type": "struct", "members": {"x": {"type": "double"}, "y": {"type": "double"}}}),
    );
    let e = xy
        .decode(&json!({"x": 1.0, "z": 0}), Mode::Request)
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert!(e.message.contains("`z`"), "{e}");

    // members come out in datainfo order, regardless of input order
    let Value::Struct(v) = xy
        .decode(&json!({"y": 2.0, "x": 1.0}), Mode::Report)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(v.keys().collect::<Vec<_>>(), ["x", "y"]);
}

// --- 4.6 commands, NaN, shapes

#[test]
fn command_arguments() {
    let DataInfo::Command(c) = info(json!({"type": "command"})) else {
        panic!()
    };
    assert_eq!(c.decode_argument(None), Ok(None));
    assert_eq!(c.decode_argument(Some(&Json::Null)), Ok(None));
    let e = c.decode_argument(Some(&json!(5))).unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert_eq!(e.path.to_string(), "argument");
    assert_eq!(c.encode_result(None), Ok(Json::Null));

    let DataInfo::Command(c) = info(
        json!({"type": "command", "argument": {"type": "bool"}, "result": {"type": "int", "min": 0, "max": 9}}),
    ) else {
        panic!()
    };
    assert_eq!(
        c.decode_argument(Some(&json!(true))),
        Ok(Some(Value::Bool(true)))
    );
    assert_eq!(
        class_of(c.decode_argument(None).map(|_| Value::Bool(true))),
        ErrorClass::WrongType
    );
    assert_eq!(c.decode_result(&json!(42)), Ok(Some(Value::Int(42))));
    assert_eq!(c.encode_result(Some(&Value::Int(3))), Ok(json!(3)));
}

#[test]
fn encode_refusals() {
    let d = info(json!({"type": "double"}));
    assert_eq!(
        d.encode(&Value::Double(f64::NAN)).unwrap_err().class,
        ErrorClass::WrongType
    );
    assert!(d.encode(&Value::Double(f64::INFINITY)).is_err());
    assert!(d.encode(&Value::Int(3)).is_err());
    let e = info(json!({"type": "enum", "members": {"a": 1}}));
    assert!(e.encode(&Value::Enum(2)).is_err());
    let s = info(json!({"type": "struct", "members": {"x": {"type": "double"}}}));
    assert!(s.encode(&Value::Struct(IndexMap::new())).is_err());
    let t = info(json!({"type": "tuple", "members": [{"type": "bool"}]}));
    assert!(t.encode(&Value::Tuple(vec![])).is_err());
}

#[test]
fn struct_round_trip() {
    let s = info(json!({"type": "struct", "members": {
        "y": {"type": "double"},
        "x": {"type": "enum", "members": {"On": 1, "Off": 0}},
        "n": {"type": "array", "maxlen": 4, "members": {"type": "blob", "maxbytes": 3}}}}));
    let first = s
        .decode(
            &json!({"x": 1, "y": 0.5, "n": ["AA==", "AQI="]}),
            Mode::Report,
        )
        .unwrap();
    let second = s.decode(&s.encode(&first).unwrap(), Mode::Report).unwrap();
    assert_eq!(first, second);
}

// --- matrix

const SPEC_BLOB: &str = "AACAPwAAAEAAAEBAAACAQAAAoEAAAMBA";

fn image() -> DataInfo {
    info(json!({"type": "matrix", "elementtype": "<f4", "names": ["x", "y"], "maxlen": [100, 100]}))
}

fn matrix_info(info: &DataInfo) -> &MatrixInfo {
    let DataInfo::Matrix(m) = info else {
        panic!("not a matrix")
    };
    m
}

#[test]
fn matrix_spec_example() {
    let m = image();
    let value = m
        .decode(&json!({"len": [2, 3], "blob": SPEC_BLOB}), Mode::Request)
        .unwrap();
    let (len, data) = value.as_matrix().unwrap();
    assert_eq!(len, [2, 3]);
    assert_eq!(data.len(), 24);
    assert_eq!(
        matrix_info(&m).to_f64s(data).unwrap(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn matrix_blob_size_does_not_fit() {
    let e = image()
        .decode(&json!({"len": [2, 3], "blob": "AACAPw=="}), Mode::Report)
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert_eq!(e.path.to_string(), "blob");
    assert!(e.to_string().contains("blob"), "{e}");
}

#[test]
fn matrix_wrong_number_of_dimensions() {
    let e = image()
        .decode(&json!({"len": [6], "blob": SPEC_BLOB}), Mode::Report)
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert_eq!(e.path.to_string(), "len");
}

#[test]
fn matrix_dimension_beyond_maxlen() {
    let blob = BASE64.encode(vec![0u8; 404]);
    let json = json!({"len": [101, 1], "blob": blob});
    let e = image().decode(&json, Mode::Request).unwrap_err();
    assert_eq!(e.class, ErrorClass::RangeError);
    assert_eq!(
        e.path.segments(),
        [PathSegment::Member("len".into()), PathSegment::Index(0)]
    );
    assert_eq!(e.path.to_string(), "len[0]");
    // reports are only checked for their shape
    assert!(image().decode(&json, Mode::Report).is_ok());
}

#[test]
fn matrix_empty() {
    let value = image()
        .decode(&json!({"len": [0, 0], "blob": ""}), Mode::Request)
        .unwrap();
    assert_eq!(value.as_matrix(), Some((&[0, 0][..], &[][..])));
    assert_eq!(
        image().encode(&value),
        Ok(json!({"len": [0, 0], "blob": ""}))
    );
    // one empty dimension makes the whole matrix empty
    assert!(
        image()
            .decode(&json!({"len": [5, 0], "blob": ""}), Mode::Request)
            .is_ok()
    );
}

#[test]
fn matrix_unknown_member() {
    let e = image()
        .decode(
            &json!({"len": [1, 1], "blob": "AACAPw==", "t": 0}),
            Mode::Report,
        )
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::WrongType);
    assert!(e.message.contains("`t`"), "{e}");
}

#[test]
fn matrix_shapes() {
    let m = image();
    for json in [
        json!([[1.0]]),
        json!({"len": [1, 1]}),
        json!({"blob": "AACAPw=="}),
        json!({"len": "1", "blob": "AACAPw=="}),
        json!({"len": [1, -1], "blob": "AACAPw=="}),
        json!({"len": [1, 1.5], "blob": "AACAPw=="}),
        json!({"len": [1, 1], "blob": [0, 0, 128, 63]}),
        json!({"len": [1, 1], "blob": "AACAPw"}),
        json!({"len": [1, 1], "blob": "AACA\nPw=="}),
        json!({"len": [4_294_967_296_u64, 1], "blob": ""}),
        json!({"len": [4_294_967_295_u32, 4_294_967_295_u32], "blob": ""}),
    ] {
        assert_eq!(
            class_of(m.decode(&json, Mode::Report)),
            ErrorClass::WrongType,
            "{json}"
        );
    }
    // the product of the lengths overflows on every platform
    let wide = info(json!({"type": "matrix", "elementtype": "<f8",
        "names": ["a", "b", "c"], "maxlen": [4_294_967_295_u32, 4_294_967_295_u32, 4_294_967_295_u32]}));
    let e = wide
        .decode(
            &json!({"len": [4_294_967_295_u32, 4_294_967_295_u32, 4_294_967_295_u32], "blob": ""}),
            Mode::Request,
        )
        .unwrap_err();
    assert_eq!(
        (e.class, e.path.to_string()),
        (ErrorClass::WrongType, "len".into())
    );
}

#[test]
fn matrix_round_trip() {
    let m = image();
    let json = json!({"len": [2, 3], "blob": SPEC_BLOB});
    let first = m.decode(&json, Mode::Report).unwrap();
    let encoded = m.encode(&first).unwrap();
    assert_eq!(encoded, json);
    assert_eq!(encoded["blob"], SPEC_BLOB);
    assert_eq!(m.decode(&encoded, Mode::Report).unwrap(), first);
}

#[test]
fn matrix_encode_checks_shape() {
    let m = image();
    let short = Value::Matrix {
        len: vec![2, 3],
        data: vec![0; 4],
    };
    let e = m.encode(&short).unwrap_err();
    assert_eq!(
        (e.class, e.path.to_string()),
        (ErrorClass::WrongType, "blob".into())
    );
    let flat = Value::Matrix {
        len: vec![6],
        data: vec![0; 24],
    };
    assert_eq!(m.encode(&flat).unwrap_err().path.to_string(), "len");
    assert!(m.encode(&Value::Blob(vec![])).is_err());
    // limits are not checked when encoding
    let big = Value::Matrix {
        len: vec![101, 1],
        data: vec![0; 404],
    };
    assert!(m.encode(&big).is_ok());
    assert_eq!(
        matrix_info(&m)
            .check(&[101, 1], 404, Mode::Request)
            .unwrap_err()
            .class,
        ErrorClass::RangeError
    );
    assert!(matrix_info(&m).check(&[101, 1], 404, Mode::Report).is_ok());
}

// --- matrix helpers

fn matrix_of(elementtype: &str) -> DataInfo {
    info(json!({"type": "matrix", "elementtype": elementtype, "names": ["x"], "maxlen": [16]}))
}

#[test]
fn matrix_helpers_both_endiannesses() {
    let values = [1.0, -2.5, 3.25];
    for t in ["<f4", ">f4", "<f8", ">f8"] {
        let info = matrix_of(t);
        let m = matrix_info(&info);
        let value = m.from_f64s(&[3], &values).unwrap();
        let (_, data) = value.as_matrix().unwrap();
        assert_eq!(data.len(), 3 * m.elementtype().size(), "{t}");
        assert_eq!(m.to_f64s(data).unwrap(), values, "{t}");
        assert_eq!(m.element_f64(data, 1), Some(-2.5), "{t}");
        assert_eq!(m.element_f64(data, 3), None, "{t}");
    }
    let le = matrix_of("<u2");
    let be = matrix_of(">u2");
    let Value::Matrix { data: le_data, .. } = matrix_info(&le).from_f64s(&[1], &[258.0]).unwrap()
    else {
        panic!()
    };
    let Value::Matrix { data: be_data, .. } = matrix_info(&be).from_f64s(&[1], &[258.0]).unwrap()
    else {
        panic!()
    };
    assert_eq!((le_data, be_data), (vec![2, 1], vec![1, 2]));
    assert_eq!(
        matrix_info(&be).element_f64(&[0xff, 0xfe], 0),
        Some(65534.0)
    );
    assert_eq!(
        matrix_info(&matrix_of(">i2")).element_f64(&[0xff, 0xfe], 0),
        Some(-2.0)
    );
    assert_eq!(
        matrix_info(&matrix_of("<i2")).element_f64(&[0xfe, 0xff], 0),
        Some(-2.0)
    );
}

#[test]
fn matrix_helpers_extremes() {
    let u8_info = matrix_of("<u1");
    let u8m = matrix_info(&u8_info);
    let v = u8m.from_f64s(&[2], &[0.0, 255.0]).unwrap();
    assert_eq!(v.as_matrix().unwrap().1, [0, 255]);
    assert_eq!(u8m.to_f64s(&[0, 255]).unwrap(), [0.0, 255.0]);
    for bad in [256.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
        let e = u8m.from_f64s(&[2], &[0.0, bad]).unwrap_err();
        assert_eq!(e.class, ErrorClass::RangeError, "{bad}");
        assert_eq!(e.path.segments(), [PathSegment::Index(1)]);
    }

    #[allow(clippy::cast_precision_loss)]
    let (min, max) = (i64::MIN as f64, i64::MAX as f64);
    for t in ["<i8", ">i8"] {
        let info = matrix_of(t);
        let m = matrix_info(&info);
        // i64::MAX as f64 is 2^63, one beyond the range
        assert!(m.from_f64s(&[1], &[max]).is_err(), "{t}");
        let v = m.from_f64s(&[2], &[min, -1.0]).unwrap();
        let (_, data) = v.as_matrix().unwrap();
        assert_eq!(m.to_f64s(data).unwrap(), [min, -1.0]);
        let mut extreme = i64::MIN.to_le_bytes();
        if t.starts_with('>') {
            extreme = i64::MIN.to_be_bytes();
        }
        assert_eq!(m.element_f64(&extreme, 0), Some(min));
    }
    let i1_info = matrix_of("<i1");
    let i1 = matrix_info(&i1_info);
    assert_eq!(i1.to_f64s(&[0x80, 0x7f]).unwrap(), [-128.0, 127.0]);
    assert!(i1.from_f64s(&[1], &[128.0]).is_err());
    assert!(i1.from_f64s(&[1], &[-128.0]).is_ok());

    let f4_info = matrix_of("<f4");
    let f4 = matrix_info(&f4_info);
    assert_eq!(
        f4.from_f64s(&[1], &[1e300]).unwrap_err().class,
        ErrorClass::RangeError
    );
    let v = f4.from_f64s(&[2], &[f64::NAN, f64::NEG_INFINITY]).unwrap();
    let back = f4.to_f64s(v.as_matrix().unwrap().1).unwrap();
    assert!(back[0].is_nan() && back[1] == f64::NEG_INFINITY);
}

#[test]
fn matrix_helpers_refusals() {
    let f2_info = matrix_of("<f2");
    let f2 = matrix_info(&f2_info);
    assert_eq!(f2.element_f64(&[0, 0x3c], 0), None);
    assert_eq!(f2.to_f64s(&[0, 0x3c]), None);
    assert_eq!(
        f2.from_f64s(&[1], &[1.0]).unwrap_err().class,
        ErrorClass::WrongType
    );
    // f2 data still passes through as bytes
    let value = f2_info
        .decode(&json!({"len": [1], "blob": "ADw="}), Mode::Request)
        .unwrap();
    assert_eq!(value.as_matrix().unwrap().1, [0, 0x3c]);

    let f4_info = matrix_of("<f4");
    let f4 = matrix_info(&f4_info);
    assert_eq!(
        f4.from_f64s(&[3], &[1.0]).unwrap_err().class,
        ErrorClass::WrongType
    );
    assert_eq!(
        f4.from_f64s(&[1, 1], &[1.0]).unwrap_err().class,
        ErrorClass::WrongType
    );
    assert_eq!(f4.to_f64s(&[0, 0, 0]), None);
    assert_eq!(MatrixInfo::element_count(&[2, 3, 4]), Some(24));
    assert_eq!(MatrixInfo::element_count(&[]), Some(1));
    assert_eq!(f4.byte_count(&[2, 3]), Some(24));
}
