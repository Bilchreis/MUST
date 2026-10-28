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

/// The C tests of the `SECoP_S_*` functions (c/server.c). They return 0 on success, else
/// the line of the failed check. The library state is global: run them one at a time.
pub mod server {
    use std::ffi::{c_char, c_int, c_ushort};

    unsafe extern "C" {
        pub fn ctest_server_basics() -> c_int;
        pub fn ctest_server_done(context: *const c_char) -> c_int;
        pub fn ctest_server_done_nodes(context: *const c_char) -> c_int;
        pub fn ctest_server_example(port: c_ushort) -> c_int;
        pub fn ctest_server_two_contexts(port1: c_ushort, port2: c_ushort) -> c_int;
        pub fn ctest_server_build_many(
            context: *const c_char,
            node: *const c_char,
            port: c_ushort,
        ) -> c_int;
        pub fn ctest_server_validation_fails(port: c_ushort) -> c_int;
        pub fn ctest_server_validation_fixed() -> c_int;
        pub fn ctest_server_callbacks(port: c_ushort) -> c_int;
        pub fn ctest_server_missing_functions() -> c_int;
        pub fn ctest_server_poll_uniform(port: c_ushort, with_parameter: c_int) -> c_int;
        pub fn ctest_server_poll_count(parameter: *const c_char) -> c_int;
        pub fn ctest_server_polling(port: c_ushort) -> c_int;
        pub fn ctest_server_poll_step(step: c_int) -> c_int;
        pub fn ctest_server_stress_build(port: c_ushort) -> c_int;
        pub fn ctest_monitor_building() -> c_int;
        pub fn ctest_status_window() -> c_int;
    }
}

/// SHALL's own test scenarios (`c/shall_ported.c`). They return 0 on success, else the line
/// of the failed check. The library state is global: run them one at a time.
pub mod shall {
    use std::ffi::{c_char, c_int, c_ushort};

    unsafe extern "C" {
        pub fn ctest_shall_node(
            context: *const c_char,
            node: *const c_char,
            port: c_ushort,
            function_pointers: c_int,
        ) -> c_int;
        pub fn ctest_shall_delete_node(node: *const c_char) -> c_int;
        pub fn ctest_shall_done(node_only: c_int, context: *const c_char) -> c_int;
        pub fn ctest_shall_get_command(context: *const c_char, expected: *const c_char) -> c_int;
    }
}
