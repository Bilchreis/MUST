// SPDX-License-Identifier: LGPL-3.0-or-later

//! Compiles the C conformance tests in c/ against include/ with the target's C compiler
//! (gcc on Linux, MSVC on Windows). `header_check.c` is compiled as C++ too.

fn main() {
    let include = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../include");
    let sources: Vec<_> = std::fs::read_dir("c")
        .expect("c/ exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    let mut c = cc::Build::new();
    c.include(&include).warnings(true).extra_warnings(true);
    if !c.get_compiler().is_like_msvc() {
        c.flag("-std=c99").flag("-pedantic");
    }
    c.files(&sources).compile("must_ctests");

    cc::Build::new()
        .cpp(true)
        .include(&include)
        .warnings(true)
        .file("c/header_check.cpp")
        .compile("must_ctests_cpp");

    println!("cargo:rerun-if-changed=c");
    println!("cargo:rerun-if-changed=../../include");
}
