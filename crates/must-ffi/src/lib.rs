// SPDX-License-Identifier: LGPL-3.0-or-later

//! The SHALL-compatible C ABI of must: SHALL's `SECoP_S_*` and `SECoP_V_*` functions,
//! built as `SHALLserver32`/`SHALLserver64`.

mod ffi_util;
pub(crate) mod variant;
mod variant_api;

/// Converts a `SECoP_S_error` code to a static description.
///
/// # Safety
/// Always safe to call; marked `unsafe` only as part of the C ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SECoP_S_getErrorString(_error: i32) -> *const std::ffi::c_char {
    c"not implemented yet".as_ptr()
}
