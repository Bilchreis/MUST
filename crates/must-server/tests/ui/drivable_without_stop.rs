// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module(Drivable)]
struct M {
    value: param!("value", double()),
    status: param!("status", status(IDLE, BUSY)),
    target: param!("target", double(), writable),
}

#[methods]
impl M {}

fn main() {}
