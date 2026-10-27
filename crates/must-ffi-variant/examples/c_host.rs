// SPDX-License-Identifier: LGPL-3.0-or-later

//! A host program like a LabVIEW VI: loads `SHALLserver32`/`64` from the directory given
//! as argument by name (as LabVIEW's Call Library Function does), builds two nodes under
//! two context IDs, prints `ready`, and runs until its standard input closes. With
//! `C_HOST_GUI=1` it asks for the status window (`bGUI`). Used by
//! `cargo xtask check` (`tests/cross_process.rs`), also as a 32-bit host read by a 64-bit
//! monitor on Windows.

use std::ffi::{CStr, c_char, c_int, c_ushort, c_void};
use std::io::{BufRead, Write};
use std::path::PathBuf;

use libloading::{Library, library_filename};

type Init = unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char) -> c_int;
type Done = unsafe extern "C" fn(c_int, *const c_char);
type CreateNode2 = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    c_ushort,
    *const c_char,
) -> c_int;
type Add = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type AddParameter = unsafe extern "C" fn(*const c_char, *const c_void, *const c_char) -> c_int;
type AddString = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> c_int;
type Complete = unsafe extern "C" fn(*const c_char) -> c_int;

fn function<T: Copy>(lib: &Library, name: &str) -> T {
    // SAFETY: the caller names the C signature of the function as T
    *unsafe { lib.get::<T>(name.as_bytes()) }.unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: c_host <dist dir>"));
    let file = dir.join(library_filename(format!("SHALLserver{}", usize::BITS)));
    // SAFETY: must's libraries have no initialisation code with preconditions
    let lib = unsafe { Library::new(&file) }.unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let init: Init = function(&lib, "SECoP_S_initLibrary");
    let done: Done = function(&lib, "SECoP_S_doneLibrary");
    let create: CreateNode2 = function(&lib, "SECoP_S_createNode2");
    let add_module: Add = function(&lib, "SECoP_S_addModule");
    let add_readable: AddParameter = function(&lib, "SECoP_S_addReadableParameter");
    let add_string: AddString = function(&lib, "SECoP_S_addPropertyString");
    let complete: Complete = function(&lib, "SECoP_S_nodeComplete");

    let gui = c_int::from(std::env::var("C_HOST_GUI").is_ok_and(|v| v == "1"));
    let contexts: [(&CStr, &CStr); 2] = [(c"lv_a", c"hostnode_a"), (c"lv_b", c"hostnode_b")];
    for (ctx, node) in contexts {
        let ctx = ctx.as_ptr();
        // SAFETY: plain C calls with NUL-terminated strings
        unsafe {
            assert_eq!(init(std::ptr::null_mut(), gui, 0, ctx), 0);
            assert_eq!(
                create(
                    node.as_ptr(),
                    c"host test".as_ptr(),
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
        }
    }
    println!("ready {}", usize::BITS);
    let _ = std::io::stdout().flush();
    // run until the parent closes our input
    for _ in std::io::stdin().lock().lines() {}
    for (ctx, _) in contexts {
        // SAFETY: NUL-terminated context
        unsafe { done(0, ctx.as_ptr()) };
    }
}
