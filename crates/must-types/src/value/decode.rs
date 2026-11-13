// SPDX-License-Identifier: LGPL-3.0-or-later

//! Decoding and checking JSON values against their datainfo.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use indexmap::IndexMap;
use serde_json::Value as Json;

use super::{
    DynLimits, Mode, Result, Value, ValueError, check_len, check_range, decode_integer, expected,
    json_kind,
};
use crate::datainfo::{
    ArrayInfo, BlobInfo, CommandInfo, DataInfo, DoubleInfo, ElementKind, EnumInfo, IntInfo,
    MatrixInfo, ScaledInfo, StringInfo, StructInfo, TupleInfo,
};
use crate::path::PathSegment;

impl DataInfo {
    /// Decodes and checks a JSON value.
    pub fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        self.decode_with(json, mode, None)
    }

    /// Decodes a value from `change` or `do` in request mode, also checking `limits` for
    /// `double`, `scaled` and `int`.
    pub fn decode_request(&self, json: &Json, limits: Option<&DynLimits>) -> Result<Value> {
        self.decode_with(json, Mode::Request, limits)
    }

    pub(super) fn decode_with(
        &self,
        json: &Json,
        mode: Mode,
        limits: Option<&DynLimits>,
    ) -> Result<Value> {
        // dynamic limits only matter for requests
        let limits = limits.filter(|_| mode == Mode::Request);
        match self {
            Self::Double(i) => i.decode(json, mode, limits),
            Self::Scaled(i) => i.decode(json, mode, limits),
            Self::Int(i) => i.decode(json, mode, limits),
            Self::Bool(_) => match json {
                Json::Bool(b) => Ok(Value::Bool(*b)),
                _ => Err(expected("true or false", json)),
            },
            Self::Enum(i) => i.decode(json),
            Self::String(i) => i.decode(json, mode),
            Self::Blob(i) => i.decode(json, mode),
            Self::Array(i) => i.decode(json, mode),
            Self::Tuple(i) => i.decode(json, mode),
            Self::Struct(i) => i.decode(json, mode),
            Self::Matrix(i) => i.decode(json, mode),
            Self::Command(_) => Err(ValueError::wrong_type(
                "a command has no value; decode its argument or result",
            )),
        }
    }
}

impl DoubleInfo {
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let value = json.as_f64().ok_or_else(|| expected("a number", json))?;
        if mode == Mode::Request {
            check_range(value, self.min(), self.max(), limits, value)?;
        }
        Ok(Value::Double(value))
    }
}

impl ScaledInfo {
    #[allow(clippy::cast_precision_loss)]
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let raw = decode_integer(json)?;
        if mode == Mode::Request {
            let (min, max) = (self.min() as f64, self.max() as f64);
            check_range(raw as f64, Some(min), Some(max), limits, raw)?;
        }
        Ok(Value::Scaled(raw))
    }
}

impl IntInfo {
    #[allow(clippy::cast_precision_loss)]
    fn decode(&self, json: &Json, mode: Mode, limits: Option<&DynLimits>) -> Result<Value> {
        let value = decode_integer(json)?;
        if mode == Mode::Request {
            let (min, max) = (self.min() as f64, self.max() as f64);
            check_range(value as f64, Some(min), Some(max), limits, value)?;
        }
        Ok(Value::Int(value))
    }
}

impl EnumInfo {
    fn decode(&self, json: &Json) -> Result<Value> {
        let value = decode_integer(json)?;
        if self.name_of(value).is_none() {
            return Err(ValueError::range(format!(
                "{value} is not a member of the enum"
            )));
        }
        Ok(Value::Enum(value))
    }
}

impl StringInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::String(s) = json else {
            return Err(expected("a string", json));
        };
        if mode == Mode::Request {
            check_len(
                s.chars().count(),
                self.minchars(),
                self.maxchars(),
                "characters",
            )?;
            if !self.is_utf8()
                && let Some(c) = s.chars().find(|c| !c.is_ascii())
            {
                return Err(ValueError::range(format!(
                    "non-ASCII character `{c}`, but the string is not declared as UTF-8"
                )));
            }
        }
        Ok(Value::String(s.clone()))
    }
}

impl BlobInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::String(s) = json else {
            return Err(expected("a base64 string", json));
        };
        let bytes = BASE64
            .decode(s)
            .map_err(|e| ValueError::wrong_type(format!("invalid base64: {e}")))?;
        if mode == Mode::Request {
            check_len(bytes.len(), self.minbytes(), Some(self.maxbytes()), "bytes")?;
        }
        Ok(Value::Blob(bytes))
    }
}

impl ArrayInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Array(items) = json else {
            return Err(expected("an array", json));
        };
        if mode == Mode::Request {
            check_len(items.len(), self.minlen(), Some(self.maxlen()), "elements")?;
        }
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                self.members()
                    .decode(item, mode)
                    .map_err(|e| e.within(PathSegment::Index(i)))
            })
            .collect::<Result<_>>()
            .map(Value::Array)
    }
}

impl TupleInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Array(items) = json else {
            return Err(expected("an array", json));
        };
        if items.len() != self.members().len() {
            return Err(ValueError::wrong_type(format!(
                "expected {} tuple members, got {}",
                self.members().len(),
                items.len()
            )));
        }
        self.members()
            .iter()
            .zip(items)
            .enumerate()
            .map(|(i, (info, item))| {
                info.decode(item, mode)
                    .map_err(|e| e.within(PathSegment::Index(i)))
            })
            .collect::<Result<_>>()
            .map(Value::Tuple)
    }
}

impl StructInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Object(items) = json else {
            return Err(expected("an object", json));
        };
        if let Some(unknown) = items.keys().find(|k| !self.members().contains_key(*k)) {
            return Err(ValueError::wrong_type(format!(
                "unknown struct member `{unknown}`"
            )));
        }
        let mut members = IndexMap::new();
        for (name, info) in self.members() {
            match items.get(name) {
                Some(item) => {
                    let value = info
                        .decode(item, mode)
                        .map_err(|e| e.within(PathSegment::Member(name.clone())))?;
                    members.insert(name.clone(), value);
                }
                None if mode == Mode::Request && self.is_optional(name) => {}
                None => {
                    return Err(ValueError::wrong_type(format!(
                        "missing struct member `{name}`"
                    )));
                }
            }
        }
        Ok(Value::Struct(members))
    }
}

const MATRIX_MEMBERS: [&str; 2] = ["len", "blob"];

fn negative_len(index: usize, len: &dyn fmt::Display) -> ValueError {
    ValueError::wrong_type(format!("expected a non-negative integer, got {len}"))
        .within(PathSegment::Index(index))
        .within(PathSegment::Member("len".into()))
}

impl MatrixInfo {
    fn decode(&self, json: &Json, mode: Mode) -> Result<Value> {
        let Json::Object(items) = json else {
            return Err(expected("an object with `len` and `blob`", json));
        };
        if let Some(unknown) = items.keys().find(|k| !MATRIX_MEMBERS.contains(&k.as_str())) {
            return Err(ValueError::wrong_type(format!(
                "unknown matrix member `{unknown}`"
            )));
        }
        let member = |name: &str| {
            items
                .get(name)
                .ok_or_else(|| ValueError::wrong_type(format!("missing matrix member `{name}`")))
        };
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        let in_blob = |e: ValueError| e.within(PathSegment::Member("blob".into()));

        let json_len = member("len")?;
        let Json::Array(json_len) = json_len else {
            return Err(in_len(expected("an array of lengths", json_len)));
        };
        let len = json_len
            .iter()
            .enumerate()
            .map(|(i, l)| l.as_u64().ok_or_else(|| negative_len(i, l)))
            .collect::<Result<Vec<u64>>>()?;
        let len = self.lengths(&len, mode)?;

        let json_blob = member("blob")?;
        let Json::String(blob) = json_blob else {
            return Err(in_blob(expected("a base64 string", json_blob)));
        };
        let data = BASE64
            .decode(blob)
            .map_err(|e| in_blob(ValueError::wrong_type(format!("invalid base64: {e}"))))?;
        self.check_bytes(&len, data.len())?;
        Ok(Value::Matrix { len, data })
    }

    /// Lengths as `u32`, checked as decoding checks the `len` member: one per dimension
    /// (`WrongType`), in [`Mode::Request`] none above its `maxlen` (`RangeError`), and
    /// each within `u32` (`WrongType`).
    pub fn lengths(&self, len: &[u64], mode: Mode) -> Result<Vec<u32>> {
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        self.check_dims(len.len()).map_err(in_len)?;
        if mode == Mode::Request {
            self.check_maxlen(len).map_err(in_len)?;
        }
        len.iter()
            .enumerate()
            .map(|(i, &l)| {
                u32::try_from(l).map_err(|_| {
                    in_len(
                        ValueError::wrong_type(format!("{l} is too long for a dimension"))
                            .within(PathSegment::Index(i)),
                    )
                })
            })
            .collect()
    }

    /// Lengths given as signed integers, as a C host holds them: a negative one is the
    /// same `WrongType` error as in JSON, the rest as [`lengths`](Self::lengths).
    pub fn signed_lengths(&self, len: &[i64], mode: Mode) -> Result<Vec<u32>> {
        let len = len
            .iter()
            .enumerate()
            .map(|(i, &l)| u64::try_from(l).map_err(|_| negative_len(i, &l)))
            .collect::<Result<Vec<u64>>>()?;
        self.lengths(&len, mode)
    }

    /// A matrix value from lengths and element bytes, checked as by
    /// [`check`](Self::check).
    pub fn value(&self, len: Vec<u32>, data: Vec<u8>, mode: Mode) -> Result<Value> {
        self.check(&len, data.len(), mode)?;
        Ok(Value::Matrix { len, data })
    }

    /// Checks lengths and the number of element bytes against the datainfo, as decoding
    /// does: one length per dimension and exactly `product(len) × element size` bytes
    /// (`WrongType`), and in [`Mode::Request`] no length above its `maxlen`
    /// (`RangeError`). Error paths point at `len` or `blob`, as for decoded JSON.
    pub fn check(&self, len: &[u32], bytes: usize, mode: Mode) -> Result<()> {
        let in_len = |e: ValueError| e.within(PathSegment::Member("len".into()));
        self.check_dims(len.len()).map_err(in_len)?;
        if mode == Mode::Request {
            let wide: Vec<u64> = len.iter().map(|&l| u64::from(l)).collect();
            self.check_maxlen(&wide).map_err(in_len)?;
        }
        self.check_bytes(len, bytes)
    }

    fn check_dims(&self, dims: usize) -> Result<()> {
        if dims == self.dims() {
            Ok(())
        } else {
            Err(ValueError::wrong_type(format!(
                "expected {} dimensions, got {dims}",
                self.dims()
            )))
        }
    }

    fn check_maxlen(&self, len: &[u64]) -> Result<()> {
        for (i, (&l, &max)) in len.iter().zip(self.maxlen()).enumerate() {
            if l > max {
                return Err(ValueError::range(format!(
                    "{l} elements along `{}`, at most {max} allowed",
                    self.names()[i]
                ))
                .within(PathSegment::Index(i)));
            }
        }
        Ok(())
    }

    fn check_bytes(&self, len: &[u32], bytes: usize) -> Result<()> {
        let Some(required) = self.byte_count(len) else {
            return Err(ValueError::wrong_type(format!(
                "the lengths {len:?} give more bytes than this platform can hold"
            ))
            .within(PathSegment::Member("len".into())));
        };
        if bytes == required {
            Ok(())
        } else {
            Err(ValueError::wrong_type(format!(
                "the lengths {len:?} require {required} bytes, got {bytes}"
            ))
            .within(PathSegment::Member("blob".into())))
        }
    }

    // --- typed access

    /// Number of elements of a matrix with lengths `len`, `None` if it overflows `usize`.
    pub fn element_count(len: &[u32]) -> Option<usize> {
        len.iter().try_fold(1usize, |count, &l| {
            count.checked_mul(usize::try_from(l).ok()?)
        })
    }

    /// Number of bytes of a matrix with lengths `len`, `None` if it overflows `usize`.
    pub fn byte_count(&self, len: &[u32]) -> Option<usize> {
        Self::element_count(len)?.checked_mul(self.elementtype().size())
    }

    /// Element `index` of `data` (in transport order) as `f64`. `None` if `index` is out
    /// of range or the elements are half floats (`f2`), which pass through as bytes only.
    /// 64-bit integers beyond ±2⁵³ lose precision.
    #[allow(clippy::cast_precision_loss)]
    pub fn element_f64(&self, data: &[u8], index: usize) -> Option<f64> {
        let t = self.elementtype();
        let size = t.size();
        let start = index.checked_mul(size)?;
        let bytes = data.get(start..start.checked_add(size)?)?;
        let mut buf = [0u8; 8];
        // place the element's bytes so that from_le/from_be of an 8-byte array works
        if t.big_endian() {
            buf[8 - size..].copy_from_slice(bytes);
        } else {
            buf[..size].copy_from_slice(bytes);
        }
        let raw = if t.big_endian() {
            u64::from_be_bytes(buf)
        } else {
            u64::from_le_bytes(buf)
        };
        let bits = u32::try_from(size * 8).ok()?;
        Some(match t.kind() {
            ElementKind::UInt => raw as f64,
            ElementKind::Int => {
                // sign-extend from the element width
                let shift = 64 - bits;
                (((raw << shift).cast_signed()) >> shift) as f64
            }
            ElementKind::Float => match size {
                4 => f64::from(f32::from_bits(u32::try_from(raw).ok()?)),
                8 => f64::from_bits(raw),
                _ => return None,
            },
        })
    }

    /// All elements of `data` as `f64`s, in transport order; `None` for half floats or a
    /// byte count that is not a multiple of the element size. See
    /// [`element_f64`](Self::element_f64).
    pub fn to_f64s(&self, data: &[u8]) -> Option<Vec<f64>> {
        let size = self.elementtype().size();
        if !data.len().is_multiple_of(size) {
            return None;
        }
        (0..data.len() / size)
            .map(|i| self.element_f64(data, i))
            .collect()
    }

    /// A matrix value with lengths `len` from elements given as `f64`s in transport order.
    ///
    /// Integer elements must be integral and within the range of the element type, float
    /// elements of `f4` must be within the `f32` range (NaN and infinities are kept);
    /// otherwise a `RangeError` points at the element's index. Half floats (`f2`) are not
    /// supported (`WrongType`); build their bytes directly. A count of values other than
    /// `product(len)`, or a wrong number of lengths, is a `WrongType` error.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn from_f64s(&self, len: &[u32], values: &[f64]) -> Result<Value> {
        let t = self.elementtype();
        let size = t.size();
        self.check_dims(len.len())
            .map_err(|e| e.within(PathSegment::Member("len".into())))?;
        if Self::element_count(len) != Some(values.len()) {
            return Err(ValueError::wrong_type(format!(
                "the lengths {len:?} do not fit {} values",
                values.len()
            )));
        }
        if t.kind() == ElementKind::Float && size == 2 {
            return Err(ValueError::wrong_type(
                "half float (`f2`) elements cannot be built from f64 values",
            ));
        }
        let bits = size * 8;
        let mut data = Vec::with_capacity(values.len() * size);
        for (i, &v) in values.iter().enumerate() {
            let out_of_range = || {
                ValueError::range(format!("{v} does not fit `{t}`")).within(PathSegment::Index(i))
            };
            let raw: u64 = match t.kind() {
                ElementKind::Float if size == 4 => {
                    let f = v as f32;
                    if v.is_finite() && f.is_infinite() {
                        return Err(out_of_range());
                    }
                    u64::from(f.to_bits())
                }
                ElementKind::Float => v.to_bits(),
                ElementKind::UInt => {
                    // 2^bits as f64 is exact; the upper limit is exclusive
                    let limit = 2f64.powi(i32::try_from(bits).unwrap_or(64));
                    if v.fract() != 0.0 || !(0.0..limit).contains(&v) {
                        return Err(out_of_range());
                    }
                    v as u64
                }
                ElementKind::Int => {
                    let limit = 2f64.powi(i32::try_from(bits - 1).unwrap_or(63));
                    if v.fract() != 0.0 || !(-limit..limit).contains(&v) {
                        return Err(out_of_range());
                    }
                    (v as i64).cast_unsigned()
                }
            };
            if t.big_endian() {
                data.extend_from_slice(&raw.to_be_bytes()[8 - size..]);
            } else {
                data.extend_from_slice(&raw.to_le_bytes()[..size]);
            }
        }
        Ok(Value::Matrix {
            len: len.to_vec(),
            data,
        })
    }
}

impl CommandInfo {
    /// Decodes the argument of a `do` request; `json` is `None` when the message has no
    /// data part. Returns `None` for a command without argument.
    pub fn decode_argument(&self, json: Option<&Json>) -> Result<Option<Value>> {
        decode_optional(self.argument(), json, Mode::Request)
            .map_err(|e| e.within(PathSegment::Argument))
    }

    /// Decodes the result in a `done` reply. Returns `None` for a command without result.
    pub fn decode_result(&self, json: &Json) -> Result<Option<Value>> {
        decode_optional(self.result(), Some(json), Mode::Report)
            .map_err(|e| e.within(PathSegment::Result))
    }

    /// Encodes the argument for a `do` request (`null` for a command without argument).
    pub fn encode_argument(&self, value: Option<&Value>) -> Result<Json> {
        encode_optional(self.argument(), value).map_err(|e| e.within(PathSegment::Argument))
    }

    /// Encodes the result for a `done` reply (`null` for a command without result).
    pub fn encode_result(&self, value: Option<&Value>) -> Result<Json> {
        encode_optional(self.result(), value).map_err(|e| e.within(PathSegment::Result))
    }
}

fn decode_optional(
    info: Option<&DataInfo>,
    json: Option<&Json>,
    mode: Mode,
) -> Result<Option<Value>> {
    match (info, json) {
        (None, None | Some(Json::Null)) => Ok(None),
        (None, Some(json)) => Err(ValueError::wrong_type(format!(
            "expected no value or null, got {}",
            json_kind(json)
        ))),
        (Some(_), None) => Err(ValueError::wrong_type("a value is required")),
        (Some(info), Some(json)) => info.decode(json, mode).map(Some),
    }
}

fn encode_optional(info: Option<&DataInfo>, value: Option<&Value>) -> Result<Json> {
    match (info, value) {
        (None, None) => Ok(Json::Null),
        (None, Some(_)) => Err(ValueError::wrong_type(
            "the command takes or returns no value",
        )),
        (Some(_), None) => Err(ValueError::wrong_type("a value is required")),
        (Some(info), Some(value)) => info.encode(value),
    }
}
