// SPDX-License-Identifier: LGPL-3.0-or-later

//! Compile errors of the module macros and the derives, with their messages pinned in
//! `tests/ui/*.stderr` (update with `TRYBUILD=overwrite` after a toolchain change).

#[test]
fn compile_errors() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
