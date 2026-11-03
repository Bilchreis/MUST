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
    fn ctest_variant_matrix() -> c_int;
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
            "variant_matrix" => ctest_variant_matrix(),
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
        pub fn ctest_server_poll_config(port: c_ushort, mode: c_int) -> c_int;
        pub fn ctest_server_poll_default_requests(port: c_ushort) -> c_int;
        pub fn ctest_server_poll_count(parameter: *const c_char) -> c_int;
        pub fn ctest_server_polling(port: c_ushort) -> c_int;
        pub fn ctest_server_poll_step(step: c_int) -> c_int;
        pub fn ctest_conc_build(port: c_ushort, kind: c_int) -> c_int;
        pub fn ctest_conc_fetch(name: *const c_char, action: c_int) -> c_int;
        pub fn ctest_conc_none() -> c_int;
        pub fn ctest_conc_answer(name: *const c_char, value: *const c_char) -> c_int;
        pub fn ctest_cb_build(port: c_ushort) -> c_int;
        pub fn ctest_cb_hang_build(port: c_ushort, pollinterval: f64) -> c_int;
        pub fn ctest_cb_calls(parameter: *const c_char) -> c_int;
        pub fn ctest_cb_release();
        pub fn ctest_server_abandoned(step: c_int, port: c_ushort) -> c_int;
        pub fn ctest_user_error_build(poll_port: c_ushort, cb_port: c_ushort) -> c_int;
        pub fn ctest_user_error_answer(which: c_int) -> c_int;
        pub fn ctest_user_error_unregister() -> c_int;
        pub fn ctest_user_error_done() -> c_int;
        pub fn ctest_filter_build(port: c_ushort, port2: c_ushort) -> c_int;
        pub fn ctest_filter_rc(
            filter: *const c_char,
            action: c_int,
            context: *const c_char,
        ) -> c_int;
        pub fn ctest_filter_take(
            filter: *const c_char,
            action: c_int,
            context: *const c_char,
            expected: *const c_char,
            expected_action: c_int,
            expected_value: *const c_char,
            name_cap: c_int,
            variant: c_int,
        ) -> c_int;
        pub fn ctest_filter_answer(value: *const c_char) -> c_int;
        pub fn ctest_check_cb_build(port: c_ushort) -> c_int;
        pub fn ctest_check_cb_answer(which: c_int) -> c_int;
        pub fn ctest_check_cb_last(
            name: *const c_char,
            value: f64,
            calls: c_int,
            setter_calls: c_int,
        ) -> c_int;
        pub fn ctest_check_cb_missing(port: c_ushort) -> c_int;
        pub fn ctest_check_poll_build(port: c_ushort) -> c_int;
        pub fn ctest_check_poll_answer(expected: f64) -> c_int;
        pub fn ctest_server_stress_build(port: c_ushort) -> c_int;
        pub fn ctest_server_matrix_callbacks(port: c_ushort) -> c_int;
        pub fn ctest_server_matrix_polling(port: c_ushort) -> c_int;
        pub fn ctest_server_matrix_step(step: c_int) -> c_int;
        pub fn ctest_monitor_building() -> c_int;
        pub fn ctest_status_window() -> c_int;
        pub fn ctest_log_build(port: c_ushort) -> c_int;
        pub fn ctest_log_message(level: c_int, text: *const c_char) -> c_int;
        pub fn ctest_log_getter_rc() -> c_int;
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
