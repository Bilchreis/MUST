// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    target: param!("setpoint", double(), writable, writable),
}

#[methods]
impl M {}

fn main() {}
