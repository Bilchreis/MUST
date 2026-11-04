// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    target: param!("setpoint", double(), writeable),
}

#[methods]
impl M {}

fn main() {}
