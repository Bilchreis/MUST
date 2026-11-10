// SPDX-License-Identifier: LGPL-3.0-or-later
use must_server::typed::prelude::*;

#[module]
struct M {
    value: param!("value", double()),
}

#[methods]
impl M {
    async fn read_vlaue(&mut self) -> Result<f64> {
        Ok(1.0)
    }
}

fn main() {}
