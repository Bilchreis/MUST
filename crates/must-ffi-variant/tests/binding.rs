// SPDX-License-Identifier: LGPL-3.0-or-later

//! Loads the built `SHALLvariant` and `SHALLserver` from `MUST_DIST_DIR` (as written by
//! `cargo xtask dist`) and checks that handles cross between them. Run by
//! `cargo xtask check`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::PathBuf;

use libloading::{Library, library_filename};

type Create = unsafe extern "C" fn(f64) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut *mut c_void);
type ToJson = unsafe extern "C" fn(*const c_void, *mut c_char, c_int) -> c_int;

fn library(name: &str) -> Library {
    let dir = PathBuf::from(std::env::var_os("MUST_DIST_DIR").expect("MUST_DIST_DIR is set"));
    let bits = usize::BITS;
    let file = dir.join(library_filename(format!("{name}{bits}")));
    // SAFETY: must's libraries have no initialisation code with preconditions
    unsafe { Library::new(&file) }.unwrap_or_else(|e| panic!("{}: {e}", file.display()))
}

fn function<T: Copy>(lib: &Library, name: &str) -> T {
    // SAFETY: the caller names the C signature of the function as T
    *unsafe { lib.get::<T>(name) }.unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn json(to_json: ToJson, handle: *const c_void) -> Option<String> {
    let mut buf = [0 as c_char; 64];
    // SAFETY: the buffer holds 64 bytes
    let n = unsafe { to_json(handle, buf.as_mut_ptr(), 64) };
    (n > 0).then(|| {
        // SAFETY: on success the buffer holds a NUL-terminated string
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

#[test]
#[ignore = "needs the dist libraries in MUST_DIST_DIR; run by cargo xtask check"]
fn handles_cross_libraries() {
    // the variant library alone: it must find SHALLserver next to itself
    let variant = library("SHALLvariant");
    let v_create: Create = function(&variant, "SECoP_V_createSimpleDouble");
    let v_destroy: Destroy = function(&variant, "SECoP_V_destroy");
    let v_to_json: ToJson = function(&variant, "SECoP_V_toJSON");
    // SAFETY: plain C call
    let mut a = unsafe { v_create(2.5) };
    assert!(!a.is_null(), "SHALLvariant could not use SHALLserver");
    assert_eq!(json(v_to_json, a).as_deref(), Some("2.5"));

    // the server library sees the same handles, and vice versa
    let server = library("SHALLserver");
    let s_create: Create = function(&server, "SECoP_V_createSimpleDouble");
    let s_to_json: ToJson = function(&server, "SECoP_V_toJSON");
    assert_eq!(json(s_to_json, a).as_deref(), Some("2.5"));
    // SAFETY: plain C call
    let mut b = unsafe { s_create(-1.0) };
    let b_copy = b;
    assert_eq!(json(v_to_json, b).as_deref(), Some("-1.0"));
    // SAFETY: valid pointers to handles
    unsafe {
        v_destroy(&raw mut b);
        v_destroy(&raw mut a);
    }
    assert!(b.is_null() && a.is_null());
    assert_eq!(
        json(s_to_json, b_copy),
        None,
        "destroyed through SHALLvariant"
    );
}
