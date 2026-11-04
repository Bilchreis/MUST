// SPDX-License-Identifier: LGPL-3.0-or-later

//! Rust values of SECoP datainfos.

use must_types::{DataInfo, ErrorClass, SecopError, Value};

/// The SECoP data types, as far as they decide which Rust types fit a datainfo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// `double`.
    Double,
    /// `scaled`.
    Scaled,
    /// `int`.
    Int,
    /// `bool`.
    Bool,
    /// `enum`.
    Enum,
    /// `string`.
    String,
    /// `blob`.
    Blob,
    /// `array`.
    Array,
    /// `tuple`.
    Tuple,
    /// `struct`.
    Struct,
}

impl Kind {
    /// The kind of a datainfo; `None` for `matrix` and `command`.
    pub fn of(info: &DataInfo) -> Option<Self> {
        Some(match info {
            DataInfo::Double(_) => Self::Double,
            DataInfo::Scaled(_) => Self::Scaled,
            DataInfo::Int(_) => Self::Int,
            DataInfo::Bool(_) => Self::Bool,
            DataInfo::Enum(_) => Self::Enum,
            DataInfo::String(_) => Self::String,
            DataInfo::Blob(_) => Self::Blob,
            DataInfo::Array(_) => Self::Array,
            DataInfo::Tuple(_) => Self::Tuple,
            DataInfo::Struct(_) => Self::Struct,
            _ => return None,
        })
    }

    /// The SECoP type name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::Scaled => "scaled",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Enum => "enum",
            Self::String => "string",
            Self::Blob => "blob",
            Self::Array => "array",
            Self::Tuple => "tuple",
            Self::Struct => "struct",
        }
    }
}

/// Whether `kinds` contains `kind`; usable in constants, for the module macro's type
/// checks.
pub const fn accepts(kinds: &[Kind], kind: Kind) -> bool {
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i] as u8 == kind as u8 {
            return true;
        }
        i += 1;
    }
    false
}

/// A Rust type that holds values of some SECoP datainfos.
///
/// Conversions take the datainfo, because some values depend on it: a `scaled` value is
/// transported as an integer, and the members of a `tuple`, `array` or `struct` have
/// datainfos of their own. Values arriving in requests have already been checked against
/// the datainfo, so a failing conversion means the datainfo and the Rust type do not
/// match, which is an `InternalError`.
pub trait SecopType: Sized + Clone + Send + Sync + 'static {
    /// The datainfo types this Rust type can hold.
    const KINDS: &'static [Kind];

    /// Whether this is `u8`, which makes `Vec<Self>` fit a `blob` too.
    const IS_BYTE: bool = false;

    /// The SECoP value of `self` for `info`.
    fn to_value(&self, info: &DataInfo) -> Result<Value, SecopError>;

    /// The Rust value of a SECoP value of `info`.
    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError>;

    /// Whether every value of `info` fits this type, e.g. whether the limits of an `int`
    /// fit a `u8`; the error says why not.
    fn fits(info: &DataInfo) -> Result<(), String> {
        let _ = info;
        Ok(())
    }

    /// The value a parameter holds before it got any: zero, false, empty.
    fn fallback() -> Self;

    /// The range of an integer type, used as the `min` and `max` of an `int` datainfo
    /// that sets none.
    fn int_range() -> Option<(i64, i64)> {
        None
    }

    /// The datainfo of the type, for types that have one of their own: the enum and
    /// struct derives.
    fn datainfo() -> Option<DataInfo> {
        None
    }

    /// `self` as a byte, for `u8`.
    fn to_byte(&self) -> Option<u8> {
        None
    }

    /// A byte as `Self`, for `u8`.
    fn from_byte(byte: u8) -> Option<Self> {
        let _ = byte;
        None
    }
}

/// The error for a value that does not match its datainfo.
pub fn mismatch(what: &str, value: &Value, info: &DataInfo) -> SecopError {
    SecopError::new(
        ErrorClass::InternalError,
        format!(
            "{what}: the value {value:?} does not fit the datainfo {}",
            info.to_json()
        ),
    )
}

/// Checks the shape of a value a module produced against its datainfo: the lengths of
/// arrays and blobs, through tuples, structs and arrays. Limits such as `min` and `max`
/// are not checked: a node may report a measured value out of range.
pub fn check_shape(value: &Value, info: &DataInfo) -> Result<(), String> {
    match (value, info) {
        (Value::Array(items), DataInfo::Array(a)) => {
            let len = items.len() as u64;
            if len > a.maxlen() || len < a.minlen() {
                return Err(format!(
                    "{len} elements, outside {}..={}",
                    a.minlen(),
                    a.maxlen()
                ));
            }
            items.iter().enumerate().try_for_each(|(i, v)| {
                check_shape(v, a.members()).map_err(|e| format!("[{i}]: {e}"))
            })
        }
        (Value::Blob(bytes), DataInfo::Blob(b)) => {
            let len = bytes.len() as u64;
            if len > b.maxbytes() || len < b.minbytes() {
                return Err(format!(
                    "{len} bytes, outside {}..={}",
                    b.minbytes(),
                    b.maxbytes()
                ));
            }
            Ok(())
        }
        (Value::Tuple(items), DataInfo::Tuple(t)) => items
            .iter()
            .zip(t.members())
            .enumerate()
            .try_for_each(|(i, (v, m))| check_shape(v, m).map_err(|e| format!("[{i}]: {e}"))),
        (Value::Struct(items), DataInfo::Struct(s)) => items.iter().try_for_each(|(k, v)| {
            s.members().get(k).map_or(Ok(()), |m| {
                check_shape(v, m).map_err(|e| format!("{k}: {e}"))
            })
        }),
        _ => Ok(()),
    }
}

/// Converts `value` for `info` and checks its shape: the SECoP value a typed module
/// publishes for `name`.
pub(crate) fn produce<T: SecopType>(
    name: &str,
    value: &T,
    info: &DataInfo,
) -> Result<Value, SecopError> {
    let v = value.to_value(info)?;
    check_shape(&v, info).map_err(|e| {
        SecopError::new(
            ErrorClass::InternalError,
            format!("the value of `{name}` does not fit its datainfo: {e}"),
        )
    })?;
    Ok(v)
}

impl SecopType for f64 {
    const KINDS: &'static [Kind] = &[Kind::Double, Kind::Scaled];

    fn to_value(&self, info: &DataInfo) -> Result<Value, SecopError> {
        match info {
            DataInfo::Scaled(s) => s.from_physical(*self).map(Value::Scaled).ok_or_else(|| {
                SecopError::new(
                    ErrorClass::InternalError,
                    format!("{self} cannot be transported with scale {}", s.scale()),
                )
            }),
            _ => Ok(Value::Double(*self)),
        }
    }

    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
        match (value, info) {
            (Value::Scaled(raw), DataInfo::Scaled(s)) => Ok(s.to_physical(*raw)),
            (Value::Double(d), _) => Ok(*d),
            #[allow(clippy::cast_precision_loss)]
            (Value::Int(i), _) => Ok(*i as Self),
            _ => Err(mismatch("f64", value, info)),
        }
    }

    fn fallback() -> Self {
        0.0
    }

    fn datainfo() -> Option<DataInfo> {
        DataInfo::double().build().ok()
    }
}

macro_rules! integer {
    ($($t:ty => $byte:expr),*) => {$(
        impl SecopType for $t {
            const KINDS: &'static [Kind] = &[Kind::Int];
            const IS_BYTE: bool = $byte;

            fn to_value(&self, _info: &DataInfo) -> Result<Value, SecopError> {
                Ok(Value::Int(i64::from(*self)))
            }

            fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
                value
                    .as_i64()
                    .and_then(|i| Self::try_from(i).ok())
                    .ok_or_else(|| mismatch(stringify!($t), value, info))
            }

            fn fits(info: &DataInfo) -> Result<(), String> {
                if let DataInfo::Int(i) = info {
                    let (min, max) = (i64::from(<$t>::MIN), i64::from(<$t>::MAX));
                    if i.min() < min || i.max() > max {
                        return Err(format!(
                            "the range {}..={} does not fit `{}` ({min}..={max})",
                            i.min(),
                            i.max(),
                            stringify!($t),
                        ));
                    }
                }
                Ok(())
            }

            fn fallback() -> Self {
                0
            }

            fn int_range() -> Option<(i64, i64)> {
                Some((i64::from(<$t>::MIN), i64::from(<$t>::MAX)))
            }

            fn datainfo() -> Option<DataInfo> {
                DataInfo::int(i64::from(<$t>::MIN), i64::from(<$t>::MAX)).build().ok()
            }

            fn to_byte(&self) -> Option<u8> {
                u8::try_from(*self).ok().filter(|_| $byte)
            }

            fn from_byte(byte: u8) -> Option<Self> {
                Self::try_from(byte).ok().filter(|_| $byte)
            }
        }
    )*};
}

integer!(i8 => false, i16 => false, i32 => false, i64 => false, u8 => true, u16 => false, u32 => false);

impl SecopType for bool {
    const KINDS: &'static [Kind] = &[Kind::Bool];

    fn to_value(&self, _info: &DataInfo) -> Result<Value, SecopError> {
        Ok(Value::Bool(*self))
    }

    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
        value.as_bool().ok_or_else(|| mismatch("bool", value, info))
    }

    fn fallback() -> Self {
        false
    }

    fn datainfo() -> Option<DataInfo> {
        DataInfo::bool().build().ok()
    }
}

impl SecopType for String {
    const KINDS: &'static [Kind] = &[Kind::String];

    fn to_value(&self, _info: &DataInfo) -> Result<Value, SecopError> {
        Ok(Value::String(self.clone()))
    }

    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| mismatch("String", value, info))
    }

    fn fallback() -> Self {
        Self::new()
    }

    fn datainfo() -> Option<DataInfo> {
        DataInfo::string().build().ok()
    }
}

impl<T: SecopType> SecopType for Vec<T> {
    const KINDS: &'static [Kind] = if T::IS_BYTE {
        &[Kind::Array, Kind::Blob]
    } else {
        &[Kind::Array]
    };

    fn to_value(&self, info: &DataInfo) -> Result<Value, SecopError> {
        match info {
            DataInfo::Blob(_) => self
                .iter()
                .map(|b| {
                    b.to_byte()
                        .ok_or_else(|| mismatch("blob", &Value::Blob(Vec::new()), info))
                })
                .collect::<Result<Vec<u8>, _>>()
                .map(Value::Blob),
            DataInfo::Array(a) => self
                .iter()
                .map(|e| e.to_value(a.members()))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Err(mismatch("Vec", &Value::Array(Vec::new()), info)),
        }
    }

    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
        match (value, info) {
            (Value::Blob(bytes), DataInfo::Blob(_)) => bytes
                .iter()
                .map(|b| T::from_byte(*b).ok_or_else(|| mismatch("blob", value, info)))
                .collect(),
            (Value::Array(items), DataInfo::Array(a)) => items
                .iter()
                .map(|e| T::from_value(e, a.members()))
                .collect(),
            _ => Err(mismatch("Vec", value, info)),
        }
    }

    fn fits(info: &DataInfo) -> Result<(), String> {
        match info {
            DataInfo::Array(a) => T::fits(a.members()),
            _ => Ok(()),
        }
    }

    fn fallback() -> Self {
        Self::new()
    }
}

macro_rules! tuple {
    ($($n:tt: $t:ident),+) => {
        impl<$($t: SecopType),+> SecopType for ($($t,)+) {
            const KINDS: &'static [Kind] = &[Kind::Tuple];

            fn to_value(&self, info: &DataInfo) -> Result<Value, SecopError> {
                let members = tuple_members(info, [$($n),+].len())?;
                Ok(Value::Tuple(vec![$(self.$n.to_value(&members[$n])?),+]))
            }

            fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
                let members = tuple_members(info, [$($n),+].len())?;
                let Value::Tuple(items) = value else {
                    return Err(mismatch("tuple", value, info));
                };
                if items.len() != members.len() {
                    return Err(mismatch("tuple", value, info));
                }
                Ok(($($t::from_value(&items[$n], &members[$n])?,)+))
            }

            fn fits(info: &DataInfo) -> Result<(), String> {
                let members = tuple_members(info, [$($n),+].len()).map_err(|e| e.message)?;
                $($t::fits(&members[$n])?;)+
                Ok(())
            }

            fn fallback() -> Self {
                ($($t::fallback(),)+)
            }

            fn datainfo() -> Option<DataInfo> {
                let members = [$($t::datainfo()?),+];
                DataInfo::tuple(members.iter()).build().ok()
            }
        }
    };
}

/// The members of a tuple datainfo with `len` members.
fn tuple_members(info: &DataInfo, len: usize) -> Result<&[DataInfo], SecopError> {
    match info {
        DataInfo::Tuple(t) if t.members().len() == len => Ok(t.members()),
        _ => Err(SecopError::new(
            ErrorClass::InternalError,
            format!(
                "expected a tuple datainfo with {len} members, got {}",
                info.to_json()
            ),
        )),
    }
}

tuple!(0: A);
tuple!(0: A, 1: B);
tuple!(0: A, 1: B, 2: C);
tuple!(0: A, 1: B, 2: C, 3: D);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H, 8: I);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H, 8: I, 9: J);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H, 8: I, 9: J, 10: K);
tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H, 8: I, 9: J, 10: K, 11: L);

/// The SECoP status codes, by name, as used in `status(...)` datainfos.
pub const STATUS_CODES: &[(&str, i64)] = &[
    ("DISABLED", 0),
    ("IDLE", 100),
    ("STANDBY", 130),
    ("PREPARED", 150),
    ("WARN", 200),
    ("WARN_STANDBY", 230),
    ("WARN_PREPARED", 250),
    ("UNSTABLE", 270),
    ("BUSY", 300),
    ("DISABLING", 310),
    ("INITIALIZING", 320),
    ("PREPARING", 340),
    ("STARTING", 360),
    ("RAMPING", 370),
    ("STABILIZING", 380),
    ("FINALIZING", 390),
    ("ERROR", 400),
    ("UNKNOWN", 401),
    ("ERROR_STANDBY", 430),
    ("ERROR_PREPARED", 450),
];

/// The value of a SECoP `status` parameter: a status code and a text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The status code, e.g. [`Status::IDLE`].
    pub code: i64,
    /// What the module is doing, for humans.
    pub text: String,
}

impl Status {
    /// `DISABLED`.
    pub const DISABLED: i64 = 0;
    /// `IDLE`.
    pub const IDLE: i64 = 100;
    /// `WARN`.
    pub const WARN: i64 = 200;
    /// `BUSY`.
    pub const BUSY: i64 = 300;
    /// `ERROR`.
    pub const ERROR: i64 = 400;

    /// A status with any code.
    pub fn new(code: i64, text: impl Into<String>) -> Self {
        Self {
            code,
            text: text.into(),
        }
    }

    /// `IDLE` with a text.
    pub fn idle(text: impl Into<String>) -> Self {
        Self::new(Self::IDLE, text)
    }

    /// `WARN` with a text.
    pub fn warn(text: impl Into<String>) -> Self {
        Self::new(Self::WARN, text)
    }

    /// `BUSY` with a text.
    pub fn busy(text: impl Into<String>) -> Self {
        Self::new(Self::BUSY, text)
    }

    /// `ERROR` with a text.
    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Self::ERROR, text)
    }

    /// Whether the code is in the BUSY range (300–399).
    pub fn is_busy(&self) -> bool {
        (300..400).contains(&self.code)
    }

    /// Whether the code is in the ERROR range (400–499).
    pub fn is_error(&self) -> bool {
        (400..500).contains(&self.code)
    }
}

impl SecopType for Status {
    const KINDS: &'static [Kind] = &[Kind::Tuple];

    fn to_value(&self, _info: &DataInfo) -> Result<Value, SecopError> {
        Ok(Value::Tuple(vec![
            Value::Enum(self.code),
            Value::String(self.text.clone()),
        ]))
    }

    fn from_value(value: &Value, info: &DataInfo) -> Result<Self, SecopError> {
        match value {
            Value::Tuple(items) => match items.as_slice() {
                [Value::Enum(code), Value::String(text)] => Ok(Self::new(*code, text.clone())),
                _ => Err(mismatch("Status", value, info)),
            },
            _ => Err(mismatch("Status", value, info)),
        }
    }

    fn fits(info: &DataInfo) -> Result<(), String> {
        match info {
            DataInfo::Tuple(t)
                if matches!(t.members(), [DataInfo::Enum(_), DataInfo::String(_)]) =>
            {
                Ok(())
            }
            _ => Err("a status is a tuple of an enum and a string".into()),
        }
    }

    fn fallback() -> Self {
        Self::idle("")
    }

    fn datainfo() -> Option<DataInfo> {
        super::Di::status(&["DISABLED", "IDLE", "WARN", "BUSY", "ERROR"])
            .build()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(clippy::needless_pass_by_value)]
    fn di(json: serde_json::Value) -> DataInfo {
        DataInfo::from_json(&json).unwrap()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn round_trip<T: SecopType + PartialEq + std::fmt::Debug>(v: T, info: &DataInfo) {
        let value = v.to_value(info).unwrap();
        assert_eq!(T::from_value(&value, info).unwrap(), v);
    }

    #[test]
    fn round_trips() {
        round_trip(4.25, &di(json!({"type": "double"})));
        round_trip(-7_i64, &di(json!({"type": "int", "min": -10, "max": 10})));
        round_trip(200_u8, &di(json!({"type": "int", "min": 0, "max": 255})));
        round_trip(true, &di(json!({"type": "bool"})));
        round_trip("A".to_owned(), &di(json!({"type": "string"})));
        round_trip(
            vec![1_u8, 2, 3],
            &di(json!({"type": "blob", "maxbytes": 8})),
        );
        round_trip(
            vec![1.5, 2.5],
            &di(json!({"type": "array", "members": {"type": "double"}, "maxlen": 4})),
        );
        round_trip(
            (1.0, "x".to_owned()),
            &di(json!({"type": "tuple", "members": [{"type": "double"}, {"type": "string"}]})),
        );
        round_trip(Status::busy("ramping"), &status_info());
    }

    #[test]
    fn scaled_is_transported_as_integer() {
        let info = di(json!({"type": "scaled", "scale": 0.1, "min": 0, "max": 1000}));
        assert_eq!(2.5.to_value(&info).unwrap(), Value::Scaled(25));
        assert_eq!(f64::from_value(&Value::Scaled(25), &info).unwrap(), 2.5);
    }

    #[test]
    fn integer_limits_must_fit() {
        let info = di(json!({"type": "int", "min": 0, "max": 300}));
        let err = u8::fits(&info).unwrap_err();
        assert!(err.contains("0..=300"), "{err}");
        assert!(i16::fits(&info).is_ok());
        assert!(u8::fits(&di(json!({"type": "int", "min": 1, "max": 2}))).is_ok());
    }

    #[test]
    fn kinds() {
        assert!(accepts(<Vec<u8>>::KINDS, Kind::Blob));
        assert!(!accepts(<Vec<i64>>::KINDS, Kind::Blob));
        assert!(accepts(f64::KINDS, Kind::Scaled));
        assert!(!accepts(String::KINDS, Kind::Double));
    }

    fn status_info() -> DataInfo {
        di(json!({"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100, "BUSY": 300}}, {"type": "string"}]}))
    }

    #[test]
    fn status_encoding() {
        let v = Status::busy("ramping").to_value(&status_info()).unwrap();
        assert_eq!(
            v,
            Value::Tuple(vec![Value::Enum(300), Value::String("ramping".into())])
        );
        assert!(Status::busy("").is_busy() && !Status::idle("").is_busy());
        assert!(Status::fits(&status_info()).is_ok());
        assert!(Status::fits(&di(json!({"type": "double"}))).is_err());
    }
}
