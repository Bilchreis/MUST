// SPDX-License-Identifier: LGPL-3.0-or-later

//! Gives the Linux shared library the SONAME of SHALL's variant library, and `$ORIGIN` as
//! run path so that it finds `SHALLserver` next to itself.

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let bits = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libSHALLvariant{bits}.so");
        println!("cargo:rustc-cdylib-link-arg=-Wl,-rpath,$ORIGIN");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
