// SPDX-License-Identifier: LGPL-3.0-or-later

//! C conformance tests of must's SHALL-compatible C ABI. The C sources in `c/` are
//! compiled by the build script and call the C functions of `must-ffi`, which is linked
//! in as a library; the tests in `tests/` run them.

// link must-ffi, which defines the C functions the C tests call
use must_ffi as _;

use std::ffi::c_int;

unsafe extern "C" {
    fn ctest_variant_create() -> c_int;
    fn ctest_variant_json() -> c_int;
    fn ctest_variant_null() -> c_int;
    fn ctest_variant_positions() -> c_int;
    fn ctest_variant_modify() -> c_int;
    fn ctest_variant_enum_struct() -> c_int;
    fn ctest_variant_additional() -> c_int;
    fn ctest_variant_compare_print() -> c_int;
}

/// Runs a C test; returns 0 on success, else the line of the failed check.
///
/// # Panics
/// For an unknown test name.
#[must_use]
pub fn run(test: &str) -> c_int {
    // SAFETY: the C tests take no arguments and only call must's C functions
    unsafe {
        match test {
            "variant_create" => ctest_variant_create(),
            "variant_json" => ctest_variant_json(),
            "variant_null" => ctest_variant_null(),
            "variant_positions" => ctest_variant_positions(),
            "variant_modify" => ctest_variant_modify(),
            "variant_enum_struct" => ctest_variant_enum_struct(),
            "variant_additional" => ctest_variant_additional(),
            "variant_compare_print" => ctest_variant_compare_print(),
            other => panic!("unknown C test {other}"),
        }
    }
}
