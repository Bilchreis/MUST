// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    value: param!("value", double() as String),
}

#[methods]
impl M {}

fn main() {}
