// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    value: param!("value", double()),
}

#[methods]
impl M {
    async fn write_value(&mut self, v: f64) -> Result<f64> {
        Ok(v)
    }
}

fn main() {}
