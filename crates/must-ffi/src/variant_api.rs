// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_V_*` C functions (`SECoP-Variant.h`).
//!
//! Every function checks its handle against the handle table and returns its documented
//! failure value for NULL or unknown handles. Positions (`uPosition`) address a pre-order
//! walk of the value, see [`Node::locate`].

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_longlong, c_uint, c_void};

use serde_json::{Value as Json, json};

use crate::ffi_util::{c_bytes, c_str, copy_out, guard, put, sized_bytes, to_c_int, to_c_uint};
use crate::variant::{
    CSECoPbaseType, Kind, Name, Node, Num, Scalar, clone_handle, destroy_handle, new_handle, vc,
    vt, with_handle,
};

/// Runs `f` on the node at `position` of a handle, allowing changes.
fn at_mut<R>(
    p: *const CSECoPbaseType,
    position: c_uint,
    f: impl FnOnce(&mut Node) -> R,
) -> Option<R> {
    with_handle(p, |root| {
        let (path, _) = root.locate(position)?;
        root.at_mut(&path).map(f)
    })
    .flatten()
}

/// Runs `f` on the node at `position` of a handle, read-only.
fn at_ref<R>(p: *const CSECoPbaseType, position: c_uint, f: impl FnOnce(&Node) -> R) -> Option<R> {
    with_handle(p, |root| {
        let (path, _) = root.locate(position)?;
        root.at(&path).map(f)
    })
    .flatten()
}

fn handle(node: Option<Node>) -> *mut CSECoPbaseType {
    node.map_or(std::ptr::null_mut(), new_handle)
}

fn json_text(json: &Json) -> Vec<u8> {
    serde_json::to_vec(json).unwrap_or_default()
}

// --- creation

/// Creates a value with default content from a datainfo description (JSON text).
///
/// # Safety
/// `szDescription` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_create(szDescription: *const c_char) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        // SAFETY: forwarded caller contract
        let json =
            unsafe { c_str(szDescription) }.and_then(|s| serde_json::from_str::<Json>(s).ok());
        handle(json.and_then(|j| Node::from_description(&j, false)))
    })
}

/// Creates a `bool` value.
///
/// # Safety
/// Always safe to call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSimpleBool(bValue: c_longlong) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        new_handle(Node::scalar(Scalar::Bool, Num::I(i64::from(bValue != 0))))
    })
}

/// Creates a `double` value without limits.
///
/// # Safety
/// Always safe to call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSimpleDouble(dValue: f64) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        new_handle(Node::scalar(
            Scalar::Double {
                min: f64::NAN,
                max: f64::NAN,
            },
            Num::F(dValue),
        ))
    })
}

/// Creates an `int` value without limits.
///
/// # Safety
/// Always safe to call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSimpleInteger(llValue: c_longlong) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        new_handle(Node::scalar(
            Scalar::Int {
                min: i64::MIN,
                max: i64::MAX,
            },
            Num::I(llValue),
        ))
    })
}

/// Creates a `scaled` value with the given scale and transported integer.
///
/// # Safety
/// Always safe to call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSimpleScaled(
    dScale: f64,
    llValue: c_longlong,
) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        if !(dScale.is_finite() && dScale > 0.0) {
            return std::ptr::null_mut();
        }
        new_handle(Node::scalar(
            Scalar::Scaled {
                scale: dScale,
                min: i64::MIN,
                max: i64::MAX,
            },
            Num::I(llValue),
        ))
    })
}

/// Creates a `string` value (empty for NULL).
///
/// # Safety
/// `szValue` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSimpleString(szValue: *const c_char) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        // SAFETY: forwarded caller contract
        let bytes = unsafe { c_bytes(szValue) }.unwrap_or_default().to_vec();
        new_handle(Node::string(bytes))
    })
}

/// Creates a SECoP status: a tuple of an enum (`IDLE` = 100) and a string.
///
/// # Safety
/// Always safe to call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_createSECoPStatus() -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        let description = json!({"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100}}, {"type": "string"}]});
        handle(Node::from_description(&description, false))
    })
}

/// Parses a value from JSON text, with the type of `pHint` if given, else inferred.
///
/// # Safety
/// `szJSON` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_fromJSON(
    szJSON: *const c_char,
    pHint: *const CSECoPbaseType,
) -> *mut CSECoPbaseType {
    guard(std::ptr::null_mut(), || {
        // SAFETY: forwarded caller contract
        let Some(json) =
            unsafe { c_str(szJSON) }.and_then(|s| serde_json::from_str::<Json>(s).ok())
        else {
            return std::ptr::null_mut();
        };
        if pHint.is_null() {
            return handle(Node::from_json(&json));
        }
        let Some(mut node) = clone_handle(pHint) else {
            return std::ptr::null_mut();
        };
        if node.import(&json, true) {
            new_handle(node)
        } else {
            std::ptr::null_mut()
        }
    })
}

/// Copies a value into a new handle stored in `*ppDst`. Returns 1 on success, 0 on error.
///
/// # Safety
/// `ppDst` must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_copy(
    ppDst: *mut *mut CSECoPbaseType,
    pSrc: *const CSECoPbaseType,
) -> c_int {
    guard(0, || {
        if ppDst.is_null() {
            return 0;
        }
        match clone_handle(pSrc) {
            Some(node) => {
                // SAFETY: non-NULL and writable per the caller's contract
                unsafe { ppDst.write(new_handle(node)) };
                1
            }
            None => 0,
        }
    })
}

/// Frees a value and sets `*ppData` to NULL.
///
/// # Safety
/// `ppData` must be NULL or point to a readable and writable handle pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_destroy(ppData: *mut *mut CSECoPbaseType) {
    guard((), || {
        if ppData.is_null() {
            return;
        }
        // SAFETY: non-NULL, readable and writable per the caller's contract
        unsafe {
            destroy_handle(*ppData);
            ppData.write(std::ptr::null_mut());
        }
    });
}

// --- conversion and printing

/// Writes the datainfo of a value as JSON into `szBuffer`; returns the needed buffer size
/// (including the NUL), or a negative value on error.
///
/// # Safety
/// `szBuffer` must be NULL or valid for `iBufferSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getType(
    pData: *const CSECoPbaseType,
    szBuffer: *mut c_char,
    iBufferSize: c_int,
) -> c_int {
    guard(-1, || {
        let Some(Some(json)) = with_handle(pData, |n| n.export_type()) else {
            return -1;
        };
        // SAFETY: forwarded caller contract
        to_c_int(unsafe {
            copy_out(
                &json_text(&json),
                szBuffer,
                usize::try_from(iBufferSize).unwrap_or(0),
            )
        })
    })
}

/// Writes the value as SECoP JSON into `szJSONBuffer`; returns the needed buffer size
/// (including the NUL), or a negative value on error.
///
/// # Safety
/// `szJSONBuffer` must be NULL or valid for `iBufferSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_toJSON(
    pData: *const CSECoPbaseType,
    szJSONBuffer: *mut c_char,
    iBufferSize: c_int,
) -> c_int {
    guard(-1, || {
        let Some(json) = with_handle(pData, |n| n.export_value()) else {
            return -1;
        };
        // SAFETY: forwarded caller contract
        to_c_int(unsafe {
            copy_out(
                &json_text(&json),
                szJSONBuffer,
                usize::try_from(iBufferSize).unwrap_or(0),
            )
        })
    })
}

unsafe extern "C" {
    fn fputs(s: *const c_char, stream: *mut c_void) -> c_int;
}

/// Prints a value to a C `FILE*`: verbosity 0 the value, 1 also its type, 2 also the
/// additional items.
///
/// The `FILE*` must come from the same C runtime as this library; on Windows, where the
/// runtime is linked statically, that is generally not the case.
///
/// # Safety
/// `pOutfile` must be NULL or a valid `FILE*` of this library's C runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_printFile(
    pOutfile: *mut c_void,
    pData: *const CSECoPbaseType,
    iVerbosity: c_int,
) {
    guard((), || {
        if pOutfile.is_null() {
            return;
        }
        let Some(text) = with_handle(pData, |n| print_text(n, iVerbosity)) else {
            return;
        };
        let Ok(text) = std::ffi::CString::new(text) else {
            return;
        };
        // SAFETY: a valid FILE* per the caller's contract
        unsafe { fputs(text.as_ptr(), pOutfile) };
    });
}

fn print_text(node: &Node, verbosity: c_int) -> String {
    let value = node.export_value();
    if verbosity <= 0 {
        return format!("{value}\n");
    }
    let mut ty = node.export_type().unwrap_or(Json::Null);
    if verbosity == 1
        && let Json::Object(map) = &mut ty
    {
        for key in node.extra.keys() {
            map.remove(key);
        }
    }
    format!("{value} {ty}\n")
}

// --- positions

/// Reports type, count and (struct member) name of the item at `uPosition`. Returns 1, or
/// 0 if the position is past the end.
///
/// # Safety
/// Output pointers must be NULL or writable. The name stays valid until the value is
/// changed or destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getInfo(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    piType: *mut c_uint,
    puCount: *mut c_uint,
    pszName: *mut *const c_char,
) -> c_int {
    guard(0, || {
        // SAFETY: forwarded caller contract
        unsafe {
            put(piType, vt::NONE);
            put(puCount, 0);
            put(pszName, std::ptr::null());
        }
        let info = with_handle(pData, |root| {
            let (path, name) = root.locate(uPosition)?;
            let node = root.at(&path)?;
            let name_ptr = name
                .and_then(|_| member_name(root, &path))
                .unwrap_or(std::ptr::null());
            Some((node.code(), node.count(), name_ptr))
        });
        let Some(Some((code, count, name))) = info else {
            return 0;
        };
        // SAFETY: forwarded caller contract
        unsafe {
            put(piType, code);
            put(puCount, to_c_uint(count));
            put(pszName, name);
        }
        1
    })
}

/// The C name of the struct member at `path`, if its parent is a struct.
fn member_name(root: &Node, path: &[usize]) -> Option<*const c_char> {
    let (last, parent_path) = path.split_last()?;
    match &root.at(parent_path)?.kind {
        Kind::Struct { items, .. } => items.get(*last).map(|(n, _)| n.as_ptr()),
        _ => None,
    }
}

/// Like [`SECoP_V_getInfo`], copying the name into `szName` (of `*puLen` bytes) and
/// setting `*puLen` to the name's length.
///
/// # Safety
/// Output pointers must be NULL or writable; `szName` valid for `*puLen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getInfo2(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    piType: *mut c_uint,
    puCount: *mut c_uint,
    szName: *mut c_char,
    puLen: *mut c_uint,
) -> c_int {
    guard(0, || {
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: forwarded caller contract
        if unsafe { SECoP_V_getInfo(pData, uPosition, piType, puCount, &raw mut name) } == 0 {
            return 0;
        }
        let bytes = if name.is_null() {
            &[][..]
        } else {
            // SAFETY: a name from getInfo is NUL-terminated
            unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes()
        };
        if !puLen.is_null() {
            // SAFETY: non-NULL and readable/writable per the caller's contract
            unsafe {
                let size = *puLen as usize;
                if !szName.is_null() && size > 0 {
                    copy_out(bytes, szName, size);
                }
                puLen.write(to_c_uint(bytes.len()));
            }
        }
        1
    })
}

// --- limits, lengths, scale

/// Sets the limits of a `double` (or array of `double`); NaN means no limit.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyMinMaxDouble(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    dblMinimum: f64,
    dblMaximum: f64,
) -> c_int {
    guard(0, || {
        if !dblMinimum.is_nan() && !dblMaximum.is_nan() && dblMinimum > dblMaximum {
            return 0;
        }
        let ok = at_mut(pData, uPosition, |n| match scalar_type_mut(n) {
            Some(Scalar::Double { min, max }) => {
                (*min, *max) = (dblMinimum, dblMaximum);
                true
            }
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

/// Sets the limits of an `int` or `scaled` (or array of them; transported integers).
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyMinMaxInteger(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    llMinimum: c_longlong,
    llMaximum: c_longlong,
) -> c_int {
    guard(0, || {
        if llMinimum > llMaximum {
            return 0;
        }
        let ok = at_mut(pData, uPosition, |n| match scalar_type_mut(n) {
            Some(Scalar::Int { min, max } | Scalar::Scaled { min, max, .. }) => {
                (*min, *max) = (llMinimum, llMaximum);
                true
            }
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

fn scalar_type_mut(node: &mut Node) -> Option<&mut Scalar> {
    match &mut node.kind {
        Kind::Scalar { ty, .. } | Kind::ScalarArray { ty, .. } => Some(ty),
        _ => None,
    }
}

fn scalar_type(node: &Node) -> Option<&Scalar> {
    match &node.kind {
        Kind::Scalar { ty, .. } | Kind::ScalarArray { ty, .. } => Some(ty),
        _ => None,
    }
}

/// Reads the limits of a `double` (or array of `double`).
///
/// # Safety
/// Output pointers must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getMinMaxDouble(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    pdblMinimum: *mut f64,
    pdblMaximum: *mut f64,
) -> c_int {
    guard(0, || {
        match at_ref(pData, uPosition, |n| scalar_type(n).cloned()) {
            Some(Some(Scalar::Double { min, max })) => {
                // SAFETY: forwarded caller contract
                unsafe {
                    put(pdblMinimum, min);
                    put(pdblMaximum, max);
                }
                1
            }
            _ => 0,
        }
    })
}

/// Reads the limits of an `int` or `scaled` (or array of them).
///
/// # Safety
/// Output pointers must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getMinMaxInteger(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    pllMinimum: *mut c_longlong,
    pllMaximum: *mut c_longlong,
) -> c_int {
    guard(0, || {
        match at_ref(pData, uPosition, |n| scalar_type(n).cloned()) {
            Some(Some(Scalar::Int { min, max } | Scalar::Scaled { min, max, .. })) => {
                // SAFETY: forwarded caller contract
                unsafe {
                    put(pllMinimum, min);
                    put(pllMaximum, max);
                }
                1
            }
            _ => 0,
        }
    })
}

/// Reads current, minimum and maximum length of an array, string or blob.
///
/// # Safety
/// Output pointers must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getArrayLength(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    puLength: *mut c_uint,
    puMinimumLength: *mut c_uint,
    puMaximumLength: *mut c_uint,
) -> c_int {
    guard(0, || {
        let lengths = at_ref(pData, uPosition, |n| match &n.kind {
            Kind::ScalarArray {
                values,
                minlen,
                maxlen,
                ..
            } => Some((values.len(), *minlen, *maxlen)),
            Kind::Bytes {
                bytes,
                minlen,
                maxlen,
                ..
            } => Some((bytes.len(), *minlen, *maxlen)),
            Kind::Array {
                items,
                minlen,
                maxlen,
                ..
            } => Some((items.len(), *minlen, *maxlen)),
            _ => None,
        });
        let Some(Some((len, min, max))) = lengths else {
            return 0;
        };
        // SAFETY: forwarded caller contract
        unsafe {
            put(puLength, to_c_uint(len));
            put(puMinimumLength, min);
            put(puMaximumLength, max);
        }
        1
    })
}

/// Sets the length of an array, string or blob; if `uMinimumLength <= uMaximumLength`
/// the limits are set first (`1, 0` changes only the length). New elements get default
/// values.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyArrayLength(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    uLength: c_uint,
    uMinimumLength: c_uint,
    uMaximumLength: c_uint,
) -> c_int {
    guard(0, || {
        let ok = at_mut(pData, uPosition, |n| {
            let set_limits = uMinimumLength <= uMaximumLength;
            let len = uLength as usize;
            let fits = |min: &mut u32, max: &mut u32| {
                if set_limits {
                    (*min, *max) = (uMinimumLength, uMaximumLength);
                }
                uLength >= *min && uLength <= *max
            };
            match &mut n.kind {
                Kind::ScalarArray {
                    ty,
                    values,
                    minlen,
                    maxlen,
                    ..
                } => {
                    let fill = ty.clone();
                    if !fits(minlen, maxlen) {
                        return false;
                    }
                    values.resize(len, fill.default_value());
                    true
                }
                Kind::Bytes {
                    bytes,
                    minlen,
                    maxlen,
                    ..
                } => {
                    if !fits(minlen, maxlen) {
                        return false;
                    }
                    bytes.resize(len, 0);
                    true
                }
                Kind::Array {
                    proto,
                    items,
                    minlen,
                    maxlen,
                } => {
                    if !fits(minlen, maxlen) {
                        return false;
                    }
                    items.resize(len, (**proto).clone());
                    true
                }
                _ => false,
            }
        });
        c_int::from(ok == Some(true))
    })
}

/// Reads the scale of a `scaled` (or array of `scaled`).
///
/// # Safety
/// `pdblValue` must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getScale(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    pdblValue: *mut f64,
) -> c_int {
    guard(0, || {
        match at_ref(pData, uPosition, |n| scalar_type(n).cloned()) {
            Some(Some(Scalar::Scaled { scale, .. })) => {
                // SAFETY: forwarded caller contract
                unsafe { put(pdblValue, scale) };
                1
            }
            _ => 0,
        }
    })
}

/// Sets the scale of a `scaled` (or array of `scaled`); it must be finite and positive.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyScale(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    dblScaleFactor: f64,
) -> c_int {
    guard(0, || {
        if !(dblScaleFactor.is_finite() && dblScaleFactor > 0.0) {
            return 0;
        }
        let ok = at_mut(pData, uPosition, |n| match scalar_type_mut(n) {
            Some(Scalar::Scaled { scale, .. }) => {
                *scale = dblScaleFactor;
                true
            }
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

// --- numbers

/// The number at `uIndex` of a scalar (index 0) or number array.
fn number_at(node: &Node, index: usize) -> Option<(Scalar, Num)> {
    match &node.kind {
        Kind::Scalar { ty, value } if index == 0 => Some((ty.clone(), *value)),
        Kind::ScalarArray { ty, values, .. } => values.get(index).map(|v| (ty.clone(), *v)),
        _ => None,
    }
}

fn set_number_at(node: &mut Node, index: usize, new: Num) -> bool {
    let (ty, slot) = match &mut node.kind {
        Kind::Scalar { ty, value } if index == 0 => (ty, value),
        Kind::ScalarArray { ty, values, .. } => match values.get_mut(index) {
            Some(slot) => (ty, slot),
            None => return false,
        },
        _ => return false,
    };
    if !ty.accepts(new) {
        return false;
    }
    *slot = new;
    true
}

/// Reads a `double` (or the physical value of a `scaled`) at `uIndex`.
///
/// # Safety
/// `pdblValue` must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getDouble(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    pdblValue: *mut f64,
) -> c_int {
    guard(0, || {
        #[allow(clippy::cast_precision_loss)]
        let value = at_ref(pData, uPosition, |n| match number_at(n, uIndex as usize) {
            Some((Scalar::Double { .. }, Num::F(f))) => Some(f),
            Some((Scalar::Scaled { scale, .. }, Num::I(i))) => Some(i as f64 * scale),
            _ => None,
        });
        match value {
            Some(Some(v)) => {
                // SAFETY: forwarded caller contract
                unsafe { put(pdblValue, v) };
                1
            }
            _ => 0,
        }
    })
}

/// Sets a `double` (or the physical value of a `scaled`, rounded) at `uIndex`, within its
/// limits.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyDouble(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    dblValue: f64,
) -> c_int {
    guard(0, || {
        let ok = at_mut(pData, uPosition, |n| {
            let new = match number_at(n, uIndex as usize) {
                Some((Scalar::Double { .. }, _)) => Num::F(dblValue),
                #[allow(clippy::cast_possible_truncation)]
                Some((Scalar::Scaled { scale, .. }, _)) => {
                    let raw = (dblValue / scale).round();
                    if !raw.is_finite() {
                        return false;
                    }
                    Num::I(raw as i64)
                }
                _ => return false,
            };
            set_number_at(n, uIndex as usize, new)
        });
        c_int::from(ok == Some(true))
    })
}

/// Reads an `int`, `bool`, `enum` or `scaled` (transported integer) at `uIndex`.
///
/// # Safety
/// `pllValue` must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getInteger(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    pllValue: *mut c_longlong,
) -> c_int {
    guard(0, || {
        let value = at_ref(pData, uPosition, |n| match number_at(n, uIndex as usize) {
            Some((_, Num::I(i))) => Some(i),
            _ => None,
        });
        match value {
            Some(Some(v)) => {
                // SAFETY: forwarded caller contract
                unsafe { put(pllValue, v) };
                1
            }
            _ => 0,
        }
    })
}

/// Sets an `int`, `bool`, `enum` or `scaled` (transported integer) at `uIndex`, within
/// its limits (or members).
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyInteger(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    llValue: c_longlong,
) -> c_int {
    guard(0, || {
        let ok = at_mut(pData, uPosition, |n| match number_at(n, uIndex as usize) {
            Some((_, Num::I(_))) => set_number_at(n, uIndex as usize, Num::I(llValue)),
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

// --- strings

/// Copies a string or blob into `pszValue` (of `uBufferSize` bytes); returns the needed
/// size including the NUL, 0 on error.
///
/// # Safety
/// `pszValue` must be NULL or valid for `uBufferSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getString(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uBufferSize: c_uint,
    pszValue: *mut c_char,
) -> c_uint {
    guard(0, || {
        if uBufferSize > 0 && pszValue.is_null() {
            return 0;
        }
        let bytes = at_ref(pData, uPosition, |n| match &n.kind {
            Kind::Bytes { bytes, .. } => Some(bytes.clone()),
            _ => None,
        });
        let Some(Some(bytes)) = bytes else { return 0 };
        // SAFETY: forwarded caller contract
        to_c_uint(unsafe { copy_out(&bytes, pszValue, uBufferSize as usize) })
    })
}

/// Replaces a string or blob with `iSize` bytes of `szValue` (NUL-terminated if
/// negative), within its length limits.
///
/// # Safety
/// `szValue` must be NULL or valid for `iSize` bytes (NUL-terminated if negative).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_modifyString(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    szValue: *const c_char,
    iSize: c_int,
) -> c_int {
    guard(0, || {
        // SAFETY: forwarded caller contract
        let new = unsafe { sized_bytes(szValue, iSize) }
            .unwrap_or_default()
            .to_vec();
        let ok = at_mut(pData, uPosition, |n| match &mut n.kind {
            Kind::Bytes {
                blob,
                bytes,
                minlen,
                maxlen,
                ..
            } => {
                let len = if *blob {
                    new.len()
                } else {
                    String::from_utf8_lossy(&new).chars().count()
                };
                if len < *minlen as usize || len > *maxlen as usize {
                    return false;
                }
                *bytes = new;
                true
            }
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

// --- enums and structs

/// Number of enum members or struct members.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getEnumStructCount(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
) -> c_uint {
    guard(0, || {
        at_ref(pData, uPosition, |n| match &n.kind {
            Kind::Struct { items, .. } => to_c_uint(items.len()),
            _ => match scalar_type(n) {
                Some(Scalar::Enum { members }) => to_c_uint(members.len()),
                _ => 0,
            },
        })
        .unwrap_or(0)
    })
}

/// Name (and value in `*pllEnumVal`) of the enum member at `uIndex`, or NULL.
///
/// # Safety
/// `pllEnumVal` must be NULL or writable. The name stays valid until the value is changed
/// or destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getEnumeration(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    pllEnumVal: *mut c_longlong,
) -> *const c_char {
    guard(std::ptr::null(), || {
        let member = at_ref(pData, uPosition, |n| match scalar_type(n) {
            Some(Scalar::Enum { members }) => members
                .get(uIndex as usize)
                .map(|(name, v)| (name.as_ptr(), *v)),
            _ => None,
        });
        match member {
            Some(Some((name, value))) => {
                // SAFETY: forwarded caller contract
                unsafe { put(pllEnumVal, value) };
                name
            }
            _ => std::ptr::null(),
        }
    })
}

/// Adds the enum member `szEnumName` = `llEnumVal`, or changes the value of an existing
/// member of that name. Fails if another member has the value.
///
/// # Safety
/// `szEnumName` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_putEnumeration(
    pData: *mut CSECoPbaseType,
    uPosition: c_uint,
    llEnumVal: c_longlong,
    szEnumName: *const c_char,
) -> c_int {
    guard(0, || {
        // SAFETY: forwarded caller contract
        let Some(name) = unsafe { c_str(szEnumName) }
            .filter(|n| !n.is_empty())
            .and_then(Name::new)
        else {
            return 0;
        };
        let ok = at_mut(pData, uPosition, |n| match scalar_type_mut(n) {
            Some(Scalar::Enum { members }) => {
                if members.iter().any(|(m, v)| *v == llEnumVal && m != &name) {
                    return false;
                }
                match members.iter_mut().find(|(m, _)| m == &name) {
                    Some((_, v)) => *v = llEnumVal,
                    None => members.push((name, llEnumVal)),
                }
                true
            }
            _ => false,
        });
        c_int::from(ok == Some(true))
    })
}

/// The struct member name of the item at `uPosition`, or NULL.
///
/// # Safety
/// The name stays valid until the value is changed or destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getStructName(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
) -> *const c_char {
    guard(std::ptr::null(), || {
        with_handle(pData, |root| {
            let (path, name) = root.locate(uPosition)?;
            name.and_then(|_| member_name(root, &path))
        })
        .flatten()
        .unwrap_or(std::ptr::null())
    })
}

/// Compares two values: `SECoP_VC_EQUAL`, `SECoP_VC_SIMILAR_VALUE` (equal values, types
/// differing only in limits or similar), `SECoP_VC_DIFF_VALUE` or `SECoP_VC_DIFF_TYPE`.
///
/// # Safety
/// Always safe to call with any handle values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_compare(
    pData1: *const CSECoPbaseType,
    pData2: *const CSECoPbaseType,
) -> c_int {
    guard(vc::DIFF_TYPE, || {
        match (clone_handle(pData1), clone_handle(pData2)) {
            (Some(a), Some(b)) => a.compare(&b),
            _ => vc::DIFF_TYPE,
        }
    })
}

// --- additional items

fn additional_at(
    p: *const CSECoPbaseType,
    position: c_uint,
    index: c_uint,
) -> Option<(String, Json)> {
    at_ref(p, position, |n| {
        n.extra
            .iter()
            .nth(index as usize)
            .map(|(k, v)| (k.clone(), v.clone()))
    })
    .flatten()
}

/// Number of additional items (datainfo properties such as `unit`) of the item at
/// `uPosition`.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getAdditionalCount(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
) -> c_uint {
    guard(0, || {
        at_ref(pData, uPosition, |n| to_c_uint(n.extra.len())).unwrap_or(0)
    })
}

/// Index of the additional item `szName` (case-insensitive), or `UINT_MAX`.
///
/// # Safety
/// `szName` must be NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getAdditionalIndex(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    szName: *const c_char,
) -> c_uint {
    guard(c_uint::MAX, || {
        // SAFETY: forwarded caller contract
        let Some(name) = (unsafe { c_str(szName) }) else {
            return c_uint::MAX;
        };
        at_ref(pData, uPosition, |n| {
            n.extra.keys().position(|k| k.eq_ignore_ascii_case(name))
        })
        .flatten()
        .map_or(c_uint::MAX, to_c_uint)
    })
}

/// Copies the key of the additional item `uIndex`; returns the needed size including the
/// NUL, 0 on error.
///
/// # Safety
/// `pszValue` must be NULL or valid for `uBufferSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getAdditionalKey(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    uBufferSize: c_uint,
    pszValue: *mut c_char,
) -> c_uint {
    guard(0, || {
        let Some((key, _)) = additional_at(pData, uPosition, uIndex) else {
            return 0;
        };
        // SAFETY: forwarded caller contract
        to_c_uint(unsafe { copy_out(key.as_bytes(), pszValue, uBufferSize as usize) })
    })
}

/// Copies the value of the additional item `uIndex` (strings as text, anything else as
/// JSON); returns the needed size including the NUL, 0 on error.
///
/// # Safety
/// `pszValue` must be NULL or valid for `uBufferSize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getAdditionalString(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
    uBufferSize: c_uint,
    pszValue: *mut c_char,
) -> c_uint {
    guard(0, || {
        let Some((_, value)) = additional_at(pData, uPosition, uIndex) else {
            return 0;
        };
        let text = match value {
            Json::String(s) => s.into_bytes(),
            other => json_text(&other),
        };
        // SAFETY: forwarded caller contract
        to_c_uint(unsafe { copy_out(&text, pszValue, uBufferSize as usize) })
    })
}

/// The additional item `uIndex` as a double; NaN if it is not a number.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_V_getAdditionalDouble(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
) -> f64 {
    guard(f64::NAN, || {
        additional_at(pData, uPosition, uIndex)
            .and_then(|(_, v)| v.as_f64())
            .unwrap_or(f64::NAN)
    })
}

/// The additional item `uIndex` as an integer; 0 if it is not a number.
///
/// # Safety
/// Always safe to call with any handle value.
#[unsafe(no_mangle)]
#[allow(clippy::cast_possible_truncation)]
pub unsafe extern "C" fn SECoP_V_getAdditionalInteger(
    pData: *const CSECoPbaseType,
    uPosition: c_uint,
    uIndex: c_uint,
) -> c_longlong {
    guard(0, || {
        additional_at(pData, uPosition, uIndex)
            .and_then(|(_, v)| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(0)
    })
}
