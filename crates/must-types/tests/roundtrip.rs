// SPDX-License-Identifier: LGPL-3.0-or-later

//! Property tests: random valid datainfos with matching values survive JSON round trips.

use must_types::{DataInfo, Mode, Value};
use proptest::prelude::*;

fn leaf() -> impl Strategy<Value = DataInfo> {
    prop_oneof![
        Just(DataInfo::double().build().unwrap()),
        (-1000.0..1000.0f64, 0.0..1000.0f64).prop_map(|(min, span)| DataInfo::double()
            .min(min)
            .max(min + span)
            .unit("K")
            .build()
            .unwrap()),
        (any::<i32>(), 0..1000i64).prop_map(|(min, span)| {
            let min = i64::from(min);
            DataInfo::int(min, min + span).build().unwrap()
        }),
        (
            prop_oneof![Just(0.1), Just(0.01), Just(2.5)],
            -100..100i64,
            0..1000i64
        )
            .prop_map(
                |(scale, min, span)| DataInfo::scaled(scale, min, min + span).build().unwrap()
            ),
        Just(DataInfo::bool().build().unwrap()),
        prop::collection::btree_map("[A-Z]{1,6}", -500..500i64, 1..5).prop_filter_map(
            "enum values must be unique",
            |members| {
                let mut values: Vec<_> = members.values().collect();
                values.sort();
                values.dedup();
                (values.len() == members.len()).then(|| {
                    DataInfo::enumeration(members.iter().map(|(k, v)| (k.as_str(), *v)))
                        .build()
                        .unwrap()
                })
            }
        ),
        any::<bool>().prop_map(|utf8| DataInfo::string().prop("isUTF8", utf8).build().unwrap()),
        (0..16u64).prop_map(|max| DataInfo::blob(max).build().unwrap()),
    ]
}

fn datainfo() -> impl Strategy<Value = DataInfo> {
    leaf().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            (inner.clone(), 0..5u64)
                .prop_map(|(member, maxlen)| DataInfo::array(&member, maxlen).build().unwrap()),
            prop::collection::vec(inner.clone(), 0..4).prop_map(|members| DataInfo::tuple(
                &members
            )
            .build()
            .unwrap()),
            prop::collection::vec(inner, 0..4).prop_map(|members| {
                let names: Vec<String> = (0..members.len()).map(|i| format!("m{i}")).collect();
                DataInfo::structure(names.iter().map(String::as_str).zip(&members))
                    .build()
                    .unwrap()
            }),
        ]
    })
}

fn value_for(info: &DataInfo) -> BoxedStrategy<Value> {
    match info {
        DataInfo::Double(_) => any::<f64>()
            .prop_filter("finite", |d| d.is_finite())
            .prop_map(Value::Double)
            .boxed(),
        DataInfo::Int(i) => (i.min()..=i.max()).prop_map(Value::Int).boxed(),
        DataInfo::Scaled(s) => (s.min()..=s.max()).prop_map(Value::Scaled).boxed(),
        DataInfo::Bool(_) => any::<bool>().prop_map(Value::Bool).boxed(),
        DataInfo::Enum(e) => {
            prop::sample::select(e.members().values().copied().collect::<Vec<_>>())
                .prop_map(Value::Enum)
                .boxed()
        }
        DataInfo::String(s) if s.is_utf8() => any::<String>().prop_map(Value::String).boxed(),
        DataInfo::String(_) => "[ -~]{0,12}".prop_map(Value::String).boxed(),
        DataInfo::Blob(b) => {
            let max = usize::try_from(b.maxbytes()).unwrap();
            prop::collection::vec(any::<u8>(), 0..=max)
                .prop_map(Value::Blob)
                .boxed()
        }
        DataInfo::Array(a) => {
            let max = usize::try_from(a.maxlen()).unwrap();
            prop::collection::vec(value_for(a.members()), 0..=max)
                .prop_map(Value::Array)
                .boxed()
        }
        DataInfo::Tuple(t) => t
            .members()
            .iter()
            .map(value_for)
            .collect::<Vec<_>>()
            .prop_map(Value::Tuple)
            .boxed(),
        DataInfo::Struct(s) => {
            let names: Vec<String> = s.members().keys().cloned().collect();
            s.members()
                .values()
                .map(value_for)
                .collect::<Vec<_>>()
                .prop_map(move |values| Value::Struct(names.iter().cloned().zip(values).collect()))
                .boxed()
        }
        other => panic!("no values for {other}"),
    }
}

fn datainfo_and_value() -> impl Strategy<Value = (DataInfo, Value)> {
    datainfo().prop_flat_map(|info| {
        let values = value_for(&info);
        (Just(info), values)
    })
}

proptest! {
    #[test]
    fn value_survives_encode_decode((info, value) in datainfo_and_value()) {
        let json = info.encode(&value).unwrap();
        // values travel as text on the wire
        let json = serde_json::from_str(&json.to_string()).unwrap();
        prop_assert_eq!(info.decode(&json, Mode::Report).unwrap(), value);
    }

    #[test]
    fn datainfo_survives_serialization(info in datainfo()) {
        let text = info.to_string();
        prop_assert_eq!(text.parse::<DataInfo>().unwrap(), info);
    }
}

/// Found by `value_survives_encode_decode`: without the `float_roundtrip` feature of `serde_json`
/// feature the last bit of this double changed on its way through JSON text.
#[test]
fn double_round_trip_is_exact() {
    let info = DataInfo::double().build().unwrap();
    for d in [
        -2.683_879_479_154_570_3e-42,
        371.628_717_019_433_9,
        f64::MIN_POSITIVE,
        0.1,
    ] {
        let text = info.encode(&Value::Double(d)).unwrap().to_string();
        let json = serde_json::from_str(&text).unwrap();
        assert_eq!(
            info.decode(&json, Mode::Report).unwrap(),
            Value::Double(d),
            "{text}"
        );
    }
}
