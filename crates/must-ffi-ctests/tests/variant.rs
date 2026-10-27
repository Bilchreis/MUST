// SPDX-License-Identifier: LGPL-3.0-or-later

//! Runs the C tests of the `SECoP_V_*` functions (c/variant.c).

fn check(test: &str) {
    let line = must_ffi_ctests::run(test);
    assert_eq!(line, 0, "C test {test} failed at c/variant.c line {line}");
}

#[test]
fn create() {
    check("variant_create");
}

#[test]
fn json() {
    check("variant_json");
}

#[test]
fn null_handles() {
    check("variant_null");
}

#[test]
fn positions() {
    check("variant_positions");
}

#[test]
fn modify() {
    check("variant_modify");
}

#[test]
fn enums_and_structs() {
    check("variant_enum_struct");
}

#[test]
fn additional_items() {
    check("variant_additional");
}

#[test]
fn compare_and_print() {
    check("variant_compare_print");
}
