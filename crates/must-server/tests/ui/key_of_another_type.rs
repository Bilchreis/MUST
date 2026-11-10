// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    value: param!("value", double(maxchars = 8)),
}

#[methods]
impl M {}

fn main() {}
