// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    target: param!("target", double(), writable),
}

#[methods]
impl M {
    async fn write_target(&mut self, t: i64) -> Result<i64> {
        Ok(t)
    }
}

fn main() {}
