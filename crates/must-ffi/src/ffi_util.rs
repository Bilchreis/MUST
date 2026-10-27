// SPDX-License-Identifier: LGPL-3.0-or-later

//! Helpers for the C functions: panic guard, C strings and SHALL's buffer conventions.

use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`, returning `fallback` if it panics, so that no panic crosses the C boundary.
pub(crate) fn guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// A C string argument as `&str`; `None` for NULL or invalid UTF-8.
///
/// # Safety
/// `s` must be NULL or point to a NUL-terminated string valid for `'a`.
pub(crate) unsafe fn c_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    // SAFETY: non-NULL and NUL-terminated per the caller's contract
    unsafe { CStr::from_ptr(s) }.to_str().ok()
}

/// A C string argument as bytes; `None` for NULL.
///
/// # Safety
/// `s` must be NULL or point to a NUL-terminated string valid for `'a`.
pub(crate) unsafe fn c_bytes<'a>(s: *const c_char) -> Option<&'a [u8]> {
    if s.is_null() {
        return None;
    }
    // SAFETY: non-NULL and NUL-terminated per the caller's contract
    Some(unsafe { CStr::from_ptr(s) }.to_bytes())
}

/// Bytes from a pointer and a length, as in SHALL's `(const char*, int size)` arguments:
/// a negative size means NUL-terminated.
///
/// # Safety
/// `s` must be NULL, or valid for `size` bytes (or NUL-terminated if `size < 0`).
pub(crate) unsafe fn sized_bytes<'a>(s: *const c_char, size: i32) -> Option<&'a [u8]> {
    if s.is_null() {
        return None;
    }
    match usize::try_from(size) {
        // SAFETY: valid for `size` bytes per the caller's contract
        Ok(len) => Some(unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) }),
        // SAFETY: NUL-terminated per the caller's contract
        Err(_) => unsafe { c_bytes(s) },
    }
}

/// SHALL's string output convention: copies as much of `data` as fits into `buffer` of
/// `size` bytes, always NUL-terminated if `size > 0`, and returns the size needed
/// including the NUL. Returns 0 if `buffer` is NULL while `size > 0`.
///
/// # Safety
/// `buffer` must be NULL or valid for writes of `size` bytes.
pub(crate) unsafe fn copy_out(data: &[u8], buffer: *mut c_char, size: usize) -> usize {
    if size > 0 {
        if buffer.is_null() {
            return 0;
        }
        let n = data.len().min(size - 1);
        // SAFETY: `buffer` holds `size` bytes and `n + 1 <= size`
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buffer.cast::<u8>(), n);
            *buffer.add(n) = 0;
        }
    }
    data.len() + 1
}

/// Writes `value` through `ptr` unless it is NULL.
///
/// # Safety
/// `ptr` must be NULL or valid for writes.
pub(crate) unsafe fn put<T>(ptr: *mut T, value: T) {
    if !ptr.is_null() {
        // SAFETY: non-NULL and writable per the caller's contract
        unsafe { ptr.write(value) };
    }
}

/// Clamps an unsigned length to the C `unsigned int` range.
pub(crate) fn to_c_uint(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Clamps a length to the C `int` range.
pub(crate) fn to_c_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}
