// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    range: property!("full scale", double(min = 0.0)),
    limit: property!("limit", double(max = range)),
}

#[methods]
impl M {}

fn main() {}
