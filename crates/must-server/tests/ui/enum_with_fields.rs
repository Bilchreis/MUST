// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[derive(Clone, SecopEnum)]
enum Mode {
    Ramp(f64),
    Pid,
}

fn main() {}
