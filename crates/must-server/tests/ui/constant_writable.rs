// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    serial: param!("serial number", string(), writable, constant = "SN-1"),
}

#[methods]
impl M {}

fn main() {}
