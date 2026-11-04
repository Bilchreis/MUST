// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

fn main() {
    let x: param!("x", double()) = 1.0;
    let _ = x;
}
