// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's `SECoP_S_*` C functions (`SECoP-exports.h`).
//!
//! Context IDs are strings; NULL is the same context as `""`. Functions without a
//! context argument find their node by name (`SECoP_S_deleteNode`, `SECoP_S_setAddFocus`,
//! `SECoP_S_updateParameter*`, `SECoP_S_logMessage`) or their request by id
//! (`SECoP_S_putCommandAnswer*`).

use std::ffi::{c_char, c_int, c_uint, c_void};

use must_types::{DataInfo, Value};
use serde_json::Value as Json;

use super::codes::err;
use super::handler::AnswerValue;
use super::state::{self, State};
use crate::ffi_util::{c_bytes, sized_bytes};
use crate::variant::{CSECoPbaseType, clone_handle};

mod building;
mod context;
mod runtime;
#[cfg(test)]
mod tests;

/// A context ID argument; NULL is the default context `""`.
///
/// # Safety
/// `s` must be NULL or a NUL-terminated string.
unsafe fn context_id(s: *const c_char) -> String {
    // SAFETY: forwarded caller contract
    unsafe { c_bytes(s) }.map_or_else(String::new, |b| String::from_utf8_lossy(b).into_owned())
}

/// A string argument, `None` for NULL.
///
/// # Safety
/// `s` must be NULL or a NUL-terminated string.
unsafe fn text(s: *const c_char) -> Option<String> {
    // SAFETY: forwarded caller contract
    unsafe { c_bytes(s) }.map(|b| String::from_utf8_lossy(b).into_owned())
}

/// JSON from SHALL's `(const char*, int size)` arguments; `Ok(None)` for NULL or empty.
/// As in SHALL, the text ends at the first NUL, so a size that counts the terminator or
/// a whole buffer works.
///
/// # Safety
/// See [`sized_bytes`].
unsafe fn sized_json(s: *const c_char, size: c_int) -> Result<Option<Json>, c_int> {
    // SAFETY: forwarded caller contract
    let text = unsafe { sized_bytes(s, size) }.map(|b| b.split(|&c| c == 0).next().unwrap_or(b));
    match text {
        None => Ok(None),
        Some(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(None),
        Some(b) => serde_json::from_slice(b)
            .map(Some)
            .map_err(|_| err::INVALID_VALUE),
    }
}

/// A copy of the node of a handle, as a value from the application; `Ok(None)` for NULL.
fn handle_value(p: *const CSECoPbaseType) -> Result<Option<AnswerValue>, c_int> {
    if p.is_null() {
        return Ok(None);
    }
    clone_handle(p)
        .map(|n| Some(AnswerValue::Node(n)))
        .ok_or(err::INVALID_VALUE)
}

/// A matrix from SHALL-style plain arguments: `dims` lengths at `len` and `size` bytes
/// at `data`, copied once. `INVALID_VALUE` for negative sizes, or NULL with a size.
///
/// # Safety
/// `len` must be NULL or valid for `dims` reads, `data` NULL or valid for `size` bytes.
unsafe fn matrix_arguments(
    len: *const c_uint,
    dims: c_int,
    data: *const c_void,
    size: c_int,
) -> Result<Value, c_int> {
    let dims = usize::try_from(dims).map_err(|_| err::INVALID_VALUE)?;
    let size = usize::try_from(size).map_err(|_| err::INVALID_VALUE)?;
    if (len.is_null() && dims > 0) || (data.is_null() && size > 0) {
        return Err(err::INVALID_VALUE);
    }
    let len = if dims == 0 {
        Vec::new()
    } else {
        // SAFETY: non-NULL and valid for `dims` reads per the caller's contract
        unsafe { std::slice::from_raw_parts(len, dims) }.to_vec()
    };
    let data = if size == 0 {
        Vec::new()
    } else {
        // SAFETY: non-NULL and valid for `size` bytes per the caller's contract
        unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec()
    };
    Ok(Value::Matrix { len, data })
}

/// Checks the lengths and byte count of a matrix against `info` before any bytes are
/// copied: `INVALID_VALUE` unless `info` is a matrix the shape fits.
///
/// # Safety
/// `len` must be NULL or valid for `dims` reads.
unsafe fn check_matrix_shape(
    info: Option<&DataInfo>,
    len: *const c_uint,
    dims: c_int,
    size: c_int,
) -> Result<(), c_int> {
    let Some(DataInfo::Matrix(m)) = info else {
        return Err(err::INVALID_VALUE);
    };
    let dims = usize::try_from(dims).map_err(|_| err::INVALID_VALUE)?;
    let size = usize::try_from(size).map_err(|_| err::INVALID_VALUE)?;
    if len.is_null() && dims > 0 {
        return Err(err::INVALID_VALUE);
    }
    let len = if dims == 0 {
        &[][..]
    } else {
        // SAFETY: non-NULL and valid for `dims` reads per the caller's contract
        unsafe { std::slice::from_raw_parts(len, dims) }
    };
    m.check(len, size, must_types::Mode::Report)
        .map_err(|_| err::INVALID_VALUE)
}

/// The value of a handle as JSON; `Ok(None)` for NULL.
fn handle_json(p: *const CSECoPbaseType) -> Result<Option<Json>, c_int> {
    if p.is_null() {
        return Ok(None);
    }
    clone_handle(p)
        .map(|n| Some(n.export_value()))
        .ok_or(err::INVALID_VALUE)
}

/// Locks the state and checks that the library is initialized.
fn initialized() -> Result<std::sync::MutexGuard<'static, State>, c_int> {
    let state = state::lock();
    if state.initialized() {
        Ok(state)
    } else {
        Err(err::NOT_INITIALIZED)
    }
}

fn code(result: Result<c_int, c_int>) -> c_int {
    result.unwrap_or_else(|e| e)
}
