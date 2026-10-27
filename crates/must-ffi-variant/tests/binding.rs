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
type Init = unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char) -> c_int;
type Done = unsafe extern "C" fn(c_int, *const c_char);
type CreateNode2 =
    unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, u16, *const c_char) -> c_int;
type Add = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type AddParameter = unsafe extern "C" fn(*const c_char, *const c_void, *const c_char) -> c_int;
type AddString = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> c_int;
type Complete = unsafe extern "C" fn(*const c_char) -> c_int;
type Update = unsafe extern "C" fn(*const c_char, *const c_void, *const c_void, f64) -> c_int;

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

#[test]
#[ignore = "needs the dist libraries in MUST_DIST_DIR; run by cargo xtask check"]
fn server_takes_handles_from_variant_library() {
    let variant = library("SHALLvariant");
    let server = library("SHALLserver");
    let v_create: Create = function(&variant, "SECoP_V_createSimpleDouble");
    let v_destroy: Destroy = function(&variant, "SECoP_V_destroy");
    let init: Init = function(&server, "SECoP_S_initLibrary");
    let done: Done = function(&server, "SECoP_S_doneLibrary");
    let create_node: CreateNode2 = function(&server, "SECoP_S_createNode2");
    let add_module: Add = function(&server, "SECoP_S_addModule");
    let add_readable: AddParameter = function(&server, "SECoP_S_addReadableParameter");
    let add_string: AddString = function(&server, "SECoP_S_addPropertyString");
    let complete: Complete = function(&server, "SECoP_S_nodeComplete");
    let update: Update = function(&server, "SECoP_S_updateParameter");

    let ctx = c"binding".as_ptr();
    // SAFETY: plain C calls with NUL-terminated strings and handles of the libraries
    unsafe {
        assert_eq!(init(std::ptr::null_mut(), 0, 0, ctx), 0);
        assert_eq!(
            create_node(
                c"bnode".as_ptr(),
                c"binding".as_ptr(),
                c"127.0.0.1".as_ptr(),
                0,
                ctx
            ),
            0
        );
        assert_eq!(add_module(c"mod1".as_ptr(), ctx), 0);
        assert_eq!(add_string(c"description".as_ptr(), c"m".as_ptr(), ctx), 0);
        assert_eq!(add_readable(c"value".as_ptr(), std::ptr::null(), ctx), 0);
        assert_eq!(add_string(c"description".as_ptr(), c"v".as_ptr(), ctx), 0);
        assert_eq!(
            add_string(c"datainfo".as_ptr(), cr#"{"type":"double"}"#.as_ptr(), ctx),
            0
        );
        assert!(complete(ctx) >= 0);

        let mut value = v_create(3.5);
        let mut sigma = v_create(0.5);
        let name = c"bnode:mod1:value".as_ptr();
        assert_eq!(update(name, value, sigma, f64::NAN), 0, "SECoP_S_SUCCESS");
        let stale = value;
        v_destroy(&raw mut value);
        v_destroy(&raw mut sigma);
        assert_eq!(
            update(name, stale, std::ptr::null(), f64::NAN),
            -13,
            "SECoP_S_ERROR_INVALID_VALUE for a destroyed handle"
        );
        done(0, ctx);
    }
}
