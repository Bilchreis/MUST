// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    stop: command!("stop"),
}

#[methods]
impl M {}

fn main() {}
