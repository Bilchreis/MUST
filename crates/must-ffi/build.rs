// SPDX-License-Identifier: LGPL-3.0-or-later

//! Gives the Linux shared library the SONAME of SHALL's library, so that programs linked
//! against `libSHALLserver64.so` find it after `xtask dist` renamed the file.

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let bits = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libSHALLserver{bits}.so");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
