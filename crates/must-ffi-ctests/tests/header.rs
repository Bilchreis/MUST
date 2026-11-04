// SPDX-License-Identifier: LGPL-3.0-or-later

//! The header checks (`c/header_check.c`, compiled as C99 and as C++11) link against
//! must-ffi: every function they call, the MUST additions included, is exported.

use std::ffi::c_int;

// links the C sources and must-ffi
use must_ffi_ctests as _;

unsafe extern "C" {
    fn run_header_example(allowed_to_loop: c_int) -> c_int;
    fn run_header_example_cpp(allowed_to_loop: c_int) -> c_int;
}

#[test]
fn header_checks_link() {
    // referencing the functions makes the linker resolve everything they call; running
    // them would start a node on SHALL's example port
    let checks: [unsafe extern "C" fn(c_int) -> c_int; 2] =
        [run_header_example, run_header_example_cpp];
    assert!(checks.iter().all(|f| (*f as usize) != 0));
}
